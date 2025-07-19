use std::process::Command;
use std::{fmt, str::from_utf8};

use clap::Parser;
use color_print::cprintln;
use redis::{self, Client, Commands};
use sqlx::{Connection, SqliteConnection, query, sqlite};

#[derive(Parser)]
#[command(version, about, long_about = None)] // Read from `Cargo.toml`
struct Cli {
    #[arg(short, long)]
    name: Option<Vec<String>>,
}

#[derive(Clone)]
enum HealthStatus {
    Healthy,
    Unhealthy,
    // add inactive status like "stall" or something
}

impl fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            HealthStatus::Healthy => "💚 Healthy",
            HealthStatus::Unhealthy => "🔴 Unhealthy",
        };

        write!(f, "{text}")
    }
}

#[derive(Clone)]
struct ContainerHealth {
    id: String,
    name: String,
    status: HealthStatus,
    container_status: String,
    cpu_percent: usize,
    memory_usage: String,
    memory_percent: usize,
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
            "{} {} {} | CPU: {:.1}% | Mem: {} ({:.1}%)",
            status_emoji,
            self.name,
            self.container_status,
            self.cpu_percent,
            self.memory_usage,
            self.memory_percent
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
            cpu_percent: 0,
            memory_usage: "0B".to_string(),
            memory_percent: 0,
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

        if container_status != "running" {
            return Self {
                id: "".to_string(),
                name: container_name.to_string(),
                status: HealthStatus::Healthy,
                container_status,
                cpu_percent: 0,
                memory_usage: "0B".to_string(),
                memory_percent: 0,
            };
        }

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
        let cpu_percent = cpu_str.trim_end_matches("%").parse::<usize>().unwrap_or(0);
        let memory_percent = mem_perc_str
            .trim_end_matches("%")
            .parse::<usize>()
            .unwrap_or(0);
        let memory_usage = mem_str;

        let status = Self::get_health_status(container_name);
        Self {
            id,
            name: container_name.to_string(),
            status,
            container_status,
            cpu_percent,
            memory_usage,
            memory_percent,
        }
    }

    pub fn refresh(&mut self) {
        *self = Self::new(&self.name);
    }

    fn get_health_status(container_name: &str) -> HealthStatus {
        HealthStatus::Healthy
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // for _ in 0..args.count {
    //     println!("Hello {}!", args.name);
    // }

    // let pool = sqlite::SqlitePoolOptions::new()
    //     .max_connections(5)
    //     .connect("sqlite://db/monitor.db")
    //     .await;

    // let container_names = cli.name.as_deref().unwrap();
    println!("🐳 Welcome to Docker Container Health Monitor!");

    let container_names = match cli.name {
        Some(names) if !names.is_empty() => names,
        _ => {
            eprintln!("No container names provided. exiting.");
            return Ok(());
        }
    };

    let mut sqlite = SqliteConnection::connect("sqlite://db/monitor.db").await?;

    // database setup
    let _setup_query = sqlx::query("
        create table if not exists containers (id text unique, name text unique, container_status text, status text);
        ").execute(&mut sqlite).await?;

    // sqlite.execute(query).unwrap();

    cprintln!("connecting to redis..");
    let redis_client = Client::open("redis://127.0.0.1/")?;
    let mut conn = redis_client.get_connection()?;
    cprintln!("<green>Redis Server Connected</green>");

    for name in container_names.clone() {
        let _: () = conn.set("health_monitor:status", true)?;

        let containers = get_containers().unwrap();

        let state_of_container = is_container_in_list(&name, containers);
        println!("container {name}: {state_of_container}");

        if !state_of_container {
            eprintln!("container {name} not found on your machine");
        } else {
            let container_info = ContainerHealth::new(&name);
            {
                let container_info = container_info.clone();
                let _add_containers_query = sqlx::query(
                    "
                insert into containers values (?,?,?,?) returning *;
                ",
                )
                .bind(container_info.id)
                .bind(container_info.name)
                .bind(container_info.container_status)
                .bind(container_info.status.to_string())
                .execute(&mut sqlite)
                .await?;
            }

            let container = ContainerHealth {
                id: container_info.id,
                name: name,
                status: HealthStatus::Healthy,
                container_status: String::from("running"),
                cpu_percent: 10,
                memory_usage: String::from("200 MB"),
                memory_percent: 6,
            };

            let _: () = conn.set_ex(
                format!("health-data:{}", container.name),
                container.fmt_health_data(),
                60,
            )?;

            println!("{}", container);
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

    // container_stats("redis")?;
    // println!("{:?}", container_names);

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

fn get_container_status(container_name: &str) -> String {
    let status_output = Command::new("docker")
        .args(&["inspect", container_name, "--format", "{{.State.Status}}"])
        .output()
        .expect("msg");

    from_utf8(&status_output.stdout).unwrap().trim().to_string()
}
