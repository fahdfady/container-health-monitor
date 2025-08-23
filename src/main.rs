use std::process::Command;
use std::{fmt, str::from_utf8};

use bollard::models::{ContainerInspectResponse, ContainerState as BollardContainerState};
use bollard::Docker;
use clap::{Parser, Subcommand};
use color_print::cprintln;
use futures_util::StreamExt;
use redis::{self, Client, Commands, RedisResult};
use serde::{Deserialize, Serialize};
use sqlx::migrate::MigrateDatabase;
use sqlx::pool::PoolConnection;
use sqlx::Sqlite;

#[derive(Parser)]
#[command(version, about, long_about = None)] // Read from `Cargo.toml`
struct Cli {
    #[command(subcommand)]
    command: CliCommands,
}

#[derive(Subcommand)]
enum CliCommands {
    /// monitor specific containers by passing their names
    Monitor {
        #[arg(short, long)]
        name: Option<Vec<String>>,

        #[arg(short, long, default_value_t = 60)]
        cache_ttl: u64, // cache time-to-live in seconds

        #[arg(short, long, default_value_t = false)]
        watch: bool,
    },

    /// monitor all container on the machine
    MonitorAll {
        #[arg(short, long, default_value_t = 60)]
        cache_ttl: u64, // cache time-to-live in seconds

        #[arg(short, long, default_value_t = false)]
        watch: bool, // BUG: adding watch here, does not watch for newly created containers, only ones which existed when starting the CLI
    },

    /// simply wipe/delete the database file for users who want to start from a clean DB
    Wipe,
}

#[derive(Clone, Serialize, Deserialize)]
enum ContainerState {
    Created,
    Running,
    Paused,
    Restarting,
    Exited,
    Stopped,
    Removing,
    Dead,
}

impl fmt::Display for ContainerState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Restarting => "restarting",
            Self::Exited => "exited",
            Self::Stopped => "stopped",
            Self::Removing => "removing",
            Self::Dead => "dead",
        };

        write!(f, "{text}")
    }
}

impl From<&Option<BollardContainerState>> for ContainerState {
    fn from(state: &Option<BollardContainerState>) -> Self {
        match state {
            Some(state) => match state.status {
                Some(bollard::models::ContainerStateStatusEnum::CREATED) => Self::Created,
                Some(bollard::models::ContainerStateStatusEnum::RUNNING) => Self::Running,
                Some(bollard::models::ContainerStateStatusEnum::PAUSED) => Self::Paused,
                Some(bollard::models::ContainerStateStatusEnum::RESTARTING) => Self::Restarting,
                Some(bollard::models::ContainerStateStatusEnum::EXITED) => Self::Exited,
                Some(bollard::models::ContainerStateStatusEnum::REMOVING) => Self::Removing,
                Some(bollard::models::ContainerStateStatusEnum::DEAD) => Self::Dead,
                _ => Self::Stopped,
            },
            None => Self::Stopped,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
enum HealthStatus {
    Healthy,
    Unhealthy,
    Stall,
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Healthy => "💚 Healthy",
            Self::Unhealthy => "🔴 Unhealthy",
            Self::Stall => "⚫ Stall",
        };

        write!(f, "{text}")
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct ContainerHealth {
    id: String,
    name: String,
    status: HealthStatus,
    container_state: ContainerState,
    restart_count: i64,
    cpu_percent: f32,
    memory_usage: String,
    memory_percent: f32,
    uptime: String,
    last_updated: i64,
}

impl fmt::Display for ContainerHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_emoji: &str = match self.container_state {
            ContainerState::Running => "🟢",
            ContainerState::Exited => "⭕",
            ContainerState::Paused => "🟡",
            ContainerState::Restarting => "🔁",
            _ => "⚪",
        };

        write!(
            f,
            "{} {} {} | CPU: {:.1}% | Mem: {} ({:.1}%) | Restarts: {} | Uptime: {} | Updated: {}s ago",
            status_emoji,
            self.name,
            self.status,
            self.cpu_percent,
            self.memory_usage,
            self.memory_percent,
            self.restart_count,
            self.uptime,
            chrono::Utc::now().timestamp() - self.last_updated
        )
    }
}

impl Default for ContainerHealth {
    fn default() -> Self {
        Self {
            id: "".to_string(),
            name: "container".to_string(),
            status: HealthStatus::Healthy,
            // need to revise what is the default State.Status of a container
            container_state: ContainerState::Exited,
            restart_count: 0,
            cpu_percent: 0.0,
            memory_usage: "0B".to_string(),
            memory_percent: 0.0,
            uptime: "".to_string(),
            last_updated: chrono::Utc::now().timestamp(),
        }
    }
}

impl ContainerHealth {
    pub async fn new(
        container_name: &str,
        docker: &Docker,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let inspect_options = bollard::query_parameters::InspectContainerOptions { size: false };

        let inspect_result: ContainerInspectResponse = docker
            .inspect_container(container_name, Some(inspect_options))
            .await?;

        // if container_state_string != "running" {}

        let container_state: ContainerState = ContainerState::from(&inspect_result.state);

        let started_at = inspect_result.state.unwrap().started_at.unwrap();

        let uptime = Self::calculate_uptime(&started_at, &container_state).unwrap();

        let restart_count = inspect_result.restart_count.unwrap();

        let stats_options = bollard::query_parameters::StatsOptions {
            one_shot: true,
            stream: false,
        };

        let stats_result = docker
            .stats(container_name, Some(stats_options))
            .into_future()
            .await
            .0
            .unwrap()?;

        let id = stats_result.clone().id.unwrap();

        let cpu_percent = Self::calculate_cpu_percent(&stats_result);
        let (memory_usage, memory_percent) = Self::calculate_memory_stats(&stats_result);

        let status = Self::get_health_status(
            container_state.to_string().as_str(),
            cpu_percent,
            memory_percent,
            restart_count,
        );

        Ok(Self {
            id,
            name: container_name.to_string(),
            status,
            container_state,
            restart_count,
            cpu_percent,
            memory_usage,
            memory_percent,
            uptime,
            last_updated: chrono::Utc::now().timestamp(),
        })
    }

    // pub fn refresh(&mut self) {
    //     *self = Self::new(&self.name);
    // }

    /// take start_time and minus the current timestamp from it, returning a formatted human-readble uptime
    fn calculate_uptime(
        started_at: &str,
        container_status: &ContainerState,
    ) -> Result<String, Box<dyn std::error::Error>> {
        match container_status {
            ContainerState::Exited => Ok("0m".to_string()),
            ContainerState::Dead => Ok("0m".to_string()),
            _ => {
                let start_time = chrono::DateTime::parse_from_rfc3339(started_at)?;
                let now_time = chrono::Utc::now();
                let duration = now_time.signed_duration_since(start_time);

                let days = duration.num_days();
                let hours = duration.num_hours() % 24;
                let minutes = duration.num_minutes() % 60;

                if days > 0 {
                    Ok(format!("{days}d {hours}h {minutes}m"))
                } else if hours > 0 {
                    Ok(format!("{hours}h {minutes}m"))
                } else {
                    Ok(format!("{minutes}m"))
                }
            }
        }
    }

    fn calculate_cpu_percent(stats: &bollard::models::ContainerStatsResponse) -> f32 {
        let cpu_stats = stats.cpu_stats.as_ref().unwrap();
        let precpu_stats = stats.precpu_stats.as_ref().unwrap();
        let cpu_usage = cpu_stats.cpu_usage.as_ref().unwrap();
        let precpu_usage = precpu_stats.cpu_usage.as_ref().unwrap();
        let system_usage = 131323 as u64; //cpu_stats.system_cpu_usage.unwrap();
        let presystem_usage = 14995 as u64; // precpu_stats.system_cpu_usage.unwrap();

        let cpu_delta =
            cpu_usage.total_usage.unwrap() as f64 - precpu_usage.total_usage.unwrap() as f64;
        let system_delta = (system_usage - presystem_usage) as f64;
        let number_cpus = cpu_usage
            .percpu_usage
            .as_ref()
            .map(|v| v.len())
            .unwrap_or(1) as f64;

        if system_delta > 0.0 && cpu_delta > 0.0 {
            ((cpu_delta / system_delta) * number_cpus * 100.0) as f32
        } else {
            0.0
        }
    }

    fn calculate_memory_stats(stats: &bollard::models::ContainerStatsResponse) -> (String, f32) {
        if let Some(memory_stats) = &stats.memory_stats {
            let usage = memory_stats.usage.unwrap_or(0);
            let limit = memory_stats.limit.unwrap_or(1);

            let memory_usage = Self::format_bytes(usage);
            let memory_percent: f32 = ((usage as f64 / limit as f64) * 100.0) as f32;

            (memory_usage, memory_percent)
        } else {
            ("0B".to_string(), 0.0)
        }
    }

    fn format_bytes(bytes: u64) -> String {
        const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];

        let mut size = bytes as f64;
        let mut unit_index = 0;

        while size >= 1024.0 && unit_index < UNITS.len() - 1 {
            size /= 1024.0;
            unit_index += 1;
        }

        if unit_index == 0 {
            format!("{}B", bytes)
        } else {
            format!("{:.1}{}", size, UNITS[unit_index])
        }
    }

    fn get_health_status(
        container_state: &str,
        cpu_percent: f32,
        memory_percent: f32,
        restart_count: i64,
    ) -> HealthStatus {
        match container_state {
            "running" => {
                if restart_count > 5 || cpu_percent > 80.0 || memory_percent > 80.0 {
                    HealthStatus::Unhealthy
                } else {
                    HealthStatus::Healthy
                }
            }
            "exited" | "dead" => HealthStatus::Unhealthy,
            "paused" => HealthStatus::Stall,
            _ => HealthStatus::Stall,
        }
    }

    fn from_cache(
        cache_key: &str,
        redis_conn: &mut redis::Connection,
    ) -> RedisResult<Option<Self>> {
        let json_data: Option<String> = redis_conn.get(cache_key)?;
        Ok(json_data
            .map(|data| serde_json::from_str(&data).expect("Failed to deserialize cached data")))
    }

    fn store_in_cache(&self, redis_conn: &mut redis::Connection, ttl: u64) -> RedisResult<()> {
        let json_data: String = serde_json::to_string(self).unwrap();

        let cache_key = format!("health-data:{}", self.name);

        let _: () = redis_conn.set_ex(cache_key, json_data, ttl)?;

        Ok(())
    }

    async fn store_in_db(&self, pool_conn: PoolConnection<Sqlite>) -> Result<(), sqlx::Error> {
        let _add_containers_query = sqlx::query(
            "
                insert or replace into containers values (?,?,?,?,?) returning *;
                ",
        )
        .bind(&self.id)
        .bind(&self.name)
        .bind(self.container_state.to_string())
        .bind(self.status.to_string())
        .bind(self.last_updated.to_string())
        .execute(&mut pool_conn.detach())
        .await?;

        Ok(())
    }

    async fn store_in_history_db(
        &self,
        pool_conn: PoolConnection<Sqlite>,
    ) -> Result<(), sqlx::Error> {
        let _add_container_history_query = sqlx::query(
            "
                insert or replace into container_history values (?,?,?,?,?) returning *;
                ",
        )
        .bind(&self.id)
        .bind(&self.name)
        .execute(&mut pool_conn.detach())
        .await?;

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    println!("🐳 Welcome to Docker Container Health Monitor!");

    let pool = setup_sqlite_db().await;

    cprintln!("🔌 Connecting to Redis...");
    let redis_client = Client::open("redis://127.0.0.1/")?;
    let redis_conn = redis_client.get_connection()?;
    cprintln!("<green>✅ Redis connected!</green>");

    match cli.command {
        CliCommands::Monitor {
            name,
            cache_ttl,
            watch,
        } => {
            match check_docker_running() {
                Ok(_) => {}
                Err(e) => {
                    cprintln!("<red>❌ Docker is not running</red>");
                    cprintln!("<red>Error:</red> {}", e);
                    return Ok(());
                }
            };

            cprintln!("<green>✅ Docker is running!</green>");
            cprintln!("<blue>Monitoring containers...</blue>");

            let container_names = match name.clone() {
                Some(names) if !names.is_empty() => names,
                _ => {
                    cprintln!("<red>no container names supplied. add names with argument --name <<NAME>></red>");
                    return Ok(());
                }
            };

            for name in &container_names {
                let state_of_container = is_container_in_list(name);
                if !state_of_container {
                    eprintln!("container {name} not found on your machine");
                }
                // else {
                // }
            }

            monitor_containers(name.unwrap(), pool, redis_conn, cache_ttl, watch).await?;
        }
        CliCommands::MonitorAll { cache_ttl, watch } => {
            match check_docker_running() {
                Ok(_) => {}
                Err(e) => {
                    cprintln!("<red>❌ Docker is not running</red>");
                    cprintln!("<red>Error:</red> {}", e);
                    return Ok(());
                }
            };
            cprintln!("<green>✅ Docker is running!</green>");
            cprintln!("<blue>Monitoring containers...</blue>");
            let container_names = get_all_containers()?;

            if container_names.is_empty() {
                cprintln!("<yellow>No containers found on your machine.</yellow>");
                return Ok(());
            }

            monitor_containers(container_names, pool, redis_conn, cache_ttl, watch).await?;
        }
        CliCommands::Wipe => {
            // delete the database file
            let db_path = std::path::Path::new("./data/monitor.db");

            // remove the db file, not the directory, user might have done something inside the dir and we don't want them to suffer.
            std::fs::remove_file(db_path)?;

            cprintln!("<green>🗑️  Successfully wiped all data</green>");
        }
    }

    Ok(())
}

async fn monitor_containers(
    container_names: Vec<String>,
    pool: sqlx::Pool<Sqlite>,
    mut redis_conn: redis::Connection,
    cache_ttl: u64,
    watch: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        for name in &container_names {
            let cache_key = format!("health-data:{}", name);

            match ContainerHealth::from_cache(&cache_key, &mut redis_conn)? {
                Some(health) => {
                    println!("(from cache) {health}");
                    continue;
                }
                None => {
                    // No cached data found, proceed to fetch fresh data
                    // Proceed to fetch fresh data even if cache retrieval fails
                    let docker = Docker::connect_with_defaults()?;
                    let container_health_info = ContainerHealth::new(name, &docker).await?;
                    let conn_2 = pool.acquire().await?;

                    container_health_info.store_in_db(conn_2).await?;
                    container_health_info.store_in_cache(&mut redis_conn, cache_ttl)?;

                    println!("{container_health_info}");
                }
            }
        }
        if !watch {
            break;
        };

        // add waiting 5 seconds for each watch (todo: review how many seconds might be appropiate)
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }

    Ok(())
}

fn get_all_containers() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let ps_output = Command::new("docker")
        .args(["ps", "-a", "--format", "{{.Names}}"])
        .output()?;

    let stdout = from_utf8(&ps_output.stdout).unwrap().trim().to_string();

    let container_names = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect();

    Ok(container_names)
}

/// takes a container name an validates if docker recognizes it
fn is_container_in_list(container_name: &str) -> bool {
    let mut stat: bool = false;

    for name in get_all_containers().unwrap() {
        if name == container_name {
            stat = true;
        }
    }

    stat
}

async fn setup_sqlite_db() -> sqlx::Pool<Sqlite> {
    use sqlx::{sqlite::SqlitePoolOptions, Sqlite};
    std::fs::create_dir_all("./data").unwrap();

    let db_path = std::path::Path::new("./data/monitor.db");

    if !Sqlite::database_exists(db_path.to_str().unwrap())
        .await
        .unwrap()
    {
        Sqlite::create_database(db_path.to_str().unwrap())
            .await
            .unwrap();
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&format!("sqlite://{}", db_path.to_str().unwrap()))
        .await
        .expect("failed to create sqlite connection pool");

    let conn_1 = pool
        .clone()
        .acquire()
        .await
        .expect("failed to acquire connection pool");

    let conn_2 = pool
        .clone()
        .acquire()
        .await
        .expect("failed to acquire connection pool");

    // database setup
    let _setup_containers_table_query = sqlx::query(
        "
        create table if not exists containers (
            id text unique,
            name text unique,
            container_state text,
            status text,
            last_updated integer
        );
        ",
    )
    .execute(&mut conn_1.detach())
    .await
    .unwrap();
    let _setup_container_history_table_query = sqlx::query(
        "
        create table if not exists container_history (
            id text unique,
            name text unique,
            status text,
            cpu_percent real,
            memory_percent real,
            restart_count text,
            uptime text,
            timestamp integer
        );
    ",
    )
    .execute(&mut conn_2.detach())
    .await
    .unwrap();

    pool
}

fn check_docker_running() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("docker")
        .arg("info")
        .output()
        .expect("failed to execute docker info command");

    if !output.status.success() {
        let error_message = from_utf8(&output.stdout).unwrap_or("Unknown error");
        return Err(Box::new(std::io::Error::other(format!(
            "Docker is not running: {error_message}"
        ))));
    }

    Ok(())
}
