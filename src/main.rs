use std::fmt;
use std::process::Command;

use clap::Parser;
use color_print::cprintln;
use redis::{self, Client, Commands};
use sqlite::{self, BindableWithIndex, State};
#[derive(Parser)]
#[command(version, about, long_about = None)] // Read from `Cargo.toml`
struct Cli {
    #[arg(short, long)]
    name: Option<Vec<String>>,
}

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

struct ContainerHealth {
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

impl ContainerHealth {
    fn health_data(&self) -> String {
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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // for _ in 0..args.count {
    //     println!("Hello {}!", args.name);
    // }

    // let container_names = cli.name.as_deref().unwrap();
    println!("🐳 Welcome to Docker Container Health Monitor!");

    if let Some(container_names) = cli.name.as_deref() {
        let sqlite =
            sqlite::open("/home/fahdashour/container-health-monitor/db/monitor.db").unwrap();
        let query = "
    create table if not exists containers (name text unique, container_status text);
    ";
        sqlite.execute(query).unwrap();

        for name in container_names {
            println!("container name: {name}");
            let add_containers_query = "
            insert into containers values (?, 'running');
            ";
            let mut statement = sqlite.prepare(add_containers_query).unwrap();
            statement.bind((1, name.as_str())).unwrap();

            while let Ok(State::Row) = statement.next() {
                println!("name = {}", statement.read::<String, _>("name").unwrap());
                println!(
                    "container_status = {}",
                    statement.read::<String, _>("container_status").unwrap()
                );
            }
        }
    }

    cprintln!("connecting to redis..");
    let redis_client = Client::open("redis://127.0.0.1/")?;
    let mut conn = redis_client.get_connection()?;
    cprintln!("<green>Redis Server Connected</green>");
    let _: () = conn.set("health_monitor:status", true)?;

    let container = ContainerHealth {
        name: String::from("sad_pare"),
        status: HealthStatus::Healthy,
        container_status: String::from("running"),
        cpu_percent: 10,
        memory_usage: String::from("200 MB"),
        memory_percent: 6,
    };

    let _: () = conn.set_ex(
        format!("health-data:{}", container.name),
        container.health_data(),
        60,
    )?;

    println!("{}", container);

    get_containers_health().unwrap();

    Ok(())
}

fn get_containers_health() -> Result<(), Box<dyn std::error::Error>> {
    let ps_output = Command::new("docker")
        .args(&["ps", "-a", "--format", "{{.Names}}"])
        .output()?;

    let stdout = std::str::from_utf8(&ps_output.stdout)
        .unwrap()
        .trim()
        .to_string();

    let container_names: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
    container_stats("redis")?;
    println!("{:?}", container_names);

    Ok(())
}

fn container_stats(container_name: &str) -> Result<ContainerHealth, Box<dyn std::error::Error>> {
    let container_status = get_container_status(container_name);

    if container_status != "running" {
        return Ok(ContainerHealth {
            name: container_name.to_string(),
            status: HealthStatus::Healthy,
            container_status,
            cpu_percent: 0,
            memory_usage: "0B".to_string(),
            memory_percent: 0,
        });
    }

    let mut binding = Command::new("docker");
    let cmd = binding.args(&["stats", "--no-stream", "format"]);
    let cpu_str = std::str::from_utf8(&cmd.args(["{{.CPUPerc}}"]).output()?.stdout)?
        .trim()
        .to_owned();

    let cpu_percent = cpu_str.trim_end_matches("%").parse::<usize>().unwrap_or(0);

    println!("CPU PERCENT {}", cpu_percent);
    let memory_percent: usize = 2;
    let memory_usage: String = String::from("eqweqwe");

    Ok(ContainerHealth {
        name: container_name.to_string(),
        status: HealthStatus::Healthy,
        container_status,
        cpu_percent,
        memory_usage: "".to_string(),
        memory_percent: 12,
    })
}

fn get_container_status(container_name: &str) -> String {
    let status_output = Command::new("docker")
        .args(&["inspect", container_name, "--format", "{{.State.Status}}"])
        .output()
        .expect("msg");

    std::str::from_utf8(&status_output.stdout)
        .unwrap()
        .trim()
        .to_string()
}

fn get_health(container: &str) -> HealthStatus {
    match container {
        "" => HealthStatus::Healthy,
        "weqwe" => HealthStatus::Unhealthy,
        _ => HealthStatus::Unhealthy,
    }
}
