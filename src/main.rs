use std::process::Command;
use std::{fmt, str::from_utf8};

use clap::{Parser, Subcommand};
use color_print::cprintln;
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
        watch: bool,
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
    restart_count: u32,
    cpu_percent: f32,
    memory_usage: String,
    memory_percent: f32,
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
            "{} {} {} | CPU: {:.1}% | Mem: {} ({:.1}%) | Restarts: {} | Updated: {}s ago",
            status_emoji,
            self.name,
            self.status,
            self.cpu_percent,
            self.memory_usage,
            self.memory_percent,
            self.restart_count,
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
            last_updated: chrono::Utc::now().timestamp(),
        }
    }
}

impl ContainerHealth {
    pub fn new(container_name: &str) -> Self {
        // runs command `docker inspect --format "{{.State.Status}}\t{{.RestartCount}}" <CONTAINER_NAME>`
        let inspect_output = Command::new("docker")
            .args([
                "inspect",
                "--format",
                "{{.State.Status}}\t{{.RestartCount}}",
                container_name,
            ])
            .output()
            .expect("Failed to inspect container");

        let inspects = from_utf8(&inspect_output.stdout)
            .expect("Failed to get container status")
            .trim()
            .split("\t")
            .collect::<Vec<&str>>();

        let container_state_string: String = inspects.first().unwrap().to_string();

        // if container_state_string != "running" {}

        let container_state = match container_state_string.as_str() {
            "running" => ContainerState::Running,
            "restarting" => ContainerState::Restarting,
            "paused" => ContainerState::Paused,
            _ => ContainerState::Exited,
        };

        let restart_count: u32 = inspects.get(1).unwrap_or(&"0").parse::<u32>().unwrap_or(0);

        // tod: convert all `docker stats` commands into one command, split it with `/t`, collect it, each line represents something we want.
        // runs command `docker stats --no-stream --format "{{.ID}}\t{{.CPUPerc}}\t{{.MemPerc}}\t{{.MemUsage}}" <CONTAINER_NAME>`
        let stats_output = Command::new("docker")
            .args([
                "stats",
                "--no-stream",
                "--format",
                "{{.ID}}\t{{.CPUPerc}}\t{{.MemPerc}}\t{{.MemUsage}}",
                container_name,
            ])
            .output()
            .expect("Failed to get container stats");

        let stats = from_utf8(&stats_output.stdout)
            .unwrap()
            .trim()
            .split("\t")
            .collect::<Vec<&str>>();

        let id = stats.first().unwrap_or(&"").to_string();

        let cpu_percent = stats.get(1).unwrap_or(&"0%").parse::<f32>().unwrap_or(0.0);

        let memory_percent = stats.get(2).unwrap_or(&"0%").parse::<f32>().unwrap_or(0.0);

        let memory_usage = stats.get(3).unwrap_or(&"0B").to_string();

        let status = Self::get_health_status(
            container_state.to_string().as_str(),
            cpu_percent,
            memory_percent,
            restart_count,
        );

        Self {
            id,
            name: container_name.to_string(),
            status,
            container_state,
            restart_count,
            cpu_percent,
            memory_usage,
            memory_percent,
            last_updated: chrono::Utc::now().timestamp(),
        }
    }

    pub fn refresh(&mut self) {
        *self = Self::new(&self.name);
    }

    fn get_health_status(
        container_state: &str,
        cpu_percent: f32,
        memory_percent: f32,
        restart_count: u32,
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

    // fn from_cache(cache_key: &str, redis_conn: &mut redis::Connection) -> RedisResult<Self> {
    //     let json_data: String = redis_conn.get(cache_key)?;

    //     let container_health: Self = serde_json::from_str(&json_data).unwrap();

    //     Ok(container_health)
    // }

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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    println!("🐳 Welcome to Docker Container Health Monitor!");

    let pool = setup_sqlite_db().await;

    let conn_1 = pool.clone().acquire().await?;

    // database setup
    let _setup_query = sqlx::query(
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
    .await?;

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
            let container_health_info = ContainerHealth::new(name);
            let conn_2 = pool.acquire().await?;

            container_health_info.store_in_db(conn_2).await?;
            container_health_info.store_in_cache(&mut redis_conn, cache_ttl)?;

            println!("{container_health_info}");
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

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&format!("sqlite://{}", db_path.to_str().unwrap()))
        .await
        .unwrap()
}
