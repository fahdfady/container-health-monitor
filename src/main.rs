use std::process::Command;
use std::{fmt, str::from_utf8};

use clap::Parser;
use color_print::cprintln;
use redis::{self, Client, Commands, RedisResult};
use serde::{Deserialize, Serialize};
use sqlx::pool::PoolConnection;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::Sqlite;

#[derive(Parser)]
#[command(version, about, long_about = None)] // Read from `Cargo.toml`
struct Cli {
    #[arg(short, long)]
    name: Option<Vec<String>>,

    #[arg(short, long)]
    cache_ttl: u64, // cache time-to-live in seconds
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
    container_status: String,
    cpu_percent: f32,
    memory_usage: String,
    memory_percent: f32,
    last_updated: i64,
}

impl fmt::Display for ContainerHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_emoji: &str = match self.container_status.as_str() {
            "running" => "🟢",
            "exited" => "🔴",
            _ => "⚪",
        };

        write!(
            f,
            "{} {} {} | CPU: {:.1}% | Mem: {} ({:.1}%) | Updated: {}s ago",
            status_emoji,
            self.name,
            self.container_status,
            self.cpu_percent,
            self.memory_usage,
            self.memory_percent,
            chrono::Utc::now().timestamp() - self.last_updated // get difference
        )
    }
}

impl Default for ContainerHealth {
    fn default() -> Self {
        Self {
            id: "".to_string(),
            name: "container".to_string(),
            status: HealthStatus::Healthy,
            container_status: "".to_string(),
            cpu_percent: 0.0,
            memory_usage: "0B".to_string(),
            memory_percent: 0.0,
            last_updated: chrono::Utc::now().timestamp(),
        }
    }
}

impl ContainerHealth {
    fn fmt_health_data(&self) -> String {
        format!(
            "{{name:{}, container_status:{}, cpu_percentage:{}, memory_usage:{}, memory_percentage:{}, snapshot_took_at:{}}}",
            self.name,
            self.container_status,
            self.cpu_percent,
            self.memory_usage,
            self.memory_percent,
            chrono::Utc::now().to_rfc3339()
        )
    }

    pub fn new(container_name: &str) -> Self {
        let status_output = Command::new("docker")
            .args(&["inspect", container_name, "--format", "{{.State.Status}}"])
            .output()
            .expect("msg");

        let container_status: String = from_utf8(&status_output.stdout).unwrap().trim().to_string();

        if container_status != "running" {}

        let mut binding = Command::new("docker");
        let cmd = binding.args(&["stats", "--no-stream", "--format"]);

        let id_str = from_utf8(&cmd.args(["{{.ID}}"]).output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned();
        let cpu_str = from_utf8(&cmd.args(["{{.CPUPerc}}"]).output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned();
        let mem_perc_str = from_utf8(&cmd.args(["{{.MemPerc}}"]).output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned();
        let mem_str = from_utf8(&cmd.args(["{{.MemUsage}}"]).output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_owned();

        let id = id_str;
        println!("{id}");
        let cpu_percent = cpu_str.trim_end_matches("%").parse::<f32>().unwrap_or(0.0);
        let memory_percent = mem_perc_str
            .trim_end_matches("%")
            .parse::<f32>()
            .unwrap_or(0.0);
        let memory_usage = mem_str;

        let status = Self::get_health_status(container_name, cpu_percent, memory_percent);
        Self {
            id,
            name: container_name.to_string(),
            status,
            container_status,
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
        container_status: &str,
        cpu_percent: f32,
        memory_percent: f32,
    ) -> HealthStatus {
        match container_status {
            "running" => {
                if cpu_percent > 80.0 || memory_percent > 80.0 {
                    HealthStatus::Unhealthy
                } else {
                    HealthStatus::Healthy
                }
            }
            "exited" | "dead" => HealthStatus::Unhealthy,
            "paused" => HealthStatus::Stall,
            _ => HealthStatus::Unhealthy,
        }
    }

    // fn from_cache(cache_key: &str, redis_conn: &mut redis::Connection) -> RedisResult<Self> {
    //     let json_data: String = redis_conn.get(cache_key)?;

    //     let container_health: Self = serde_json::from_str(&json_data).unwrap();

    //     Ok(container_health)
    // }

    fn store_in_cache(&self, redis_conn: &mut redis::Connection, ttl: u64) -> RedisResult<()> {
        let json_data: String = serde_json::to_string(self).unwrap();
        println!("{json_data}");

        let cache_key = format!("health-data:{}", self.name);

        let _: () = redis_conn.set_ex(cache_key, json_data, ttl)?;

        Ok(())
    }

    async fn store_in_db(&self, mut db_conn: PoolConnection<Sqlite>) -> Result<(), sqlx::Error> {
        let _add_containers_query = sqlx::query(
            "
                insert or replace into containers values (?,?,?,?,?) returning *;
                ",
        )
        .bind(&self.id)
        .bind(&self.name)
        .bind(&self.container_status)
        .bind(self.status.to_string())
        .bind(self.last_updated.to_string())
        .execute(&mut db_conn.detach())
        .await?;

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    println!("🐳 Welcome to Docker Container Health Monitor!");

    let container_names = match cli.name {
        Some(names) if !names.is_empty() => names,
        _ => {
            eprintln!("No container names provided. use --name <NAME> argument.");
            return Ok(());
        }
    };
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .min_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect("sqlite://db/monitor.db")
        .await?;

    let conn_1 = pool.clone().acquire().await?;

    // database setup
    let _setup_query = sqlx::query(
        "
        create table if not exists containers (
            id text unique,
            name text unique,
            container_status text,
            status text,
            last_updated integer
        );
        ",
    )
    .execute(&mut conn_1.detach())
    .await?;

    cprintln!("🔌 Connecting to Redis...");
    let redis_client = Client::open("redis://127.0.0.1/")?;
    let mut redis_conn = redis_client.get_connection()?;
    cprintln!("<green>✅ Redis connected!</green>");

    for name in &container_names {
        let containers = get_containers().unwrap();

        let state_of_container = is_container_in_list(&name, containers);
        println!("container {name}: {state_of_container}");

        if !state_of_container {
            eprintln!("container {name} not found on your machine");
        } else {
            let container_info = ContainerHealth::new(&name);
            let conn_2 = pool.acquire().await?;

            container_info.store_in_db(conn_2).await?;
            container_info.store_in_cache(&mut redis_conn, cli.cache_ttl)?;
        }
    }

    Ok(())
}

fn get_containers() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let ps_output = Command::new("docker")
        .args(&["ps", "-a", "--format", "{{.Names}}"])
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
fn is_container_in_list(container_name: &str, containers_list: Vec<String>) -> bool {
    let mut stat: bool = false;

    for name in containers_list {
        if name == container_name {
            stat = true;
        }
    }

    stat
}
