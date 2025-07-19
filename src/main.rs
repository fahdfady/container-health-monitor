use std::process::Command;
use std::{fmt, str::from_utf8};

use clap::Parser;
use color_print::cprintln;
use redis::{self, Client, Commands};
use sqlite::{self, State};
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

    pub fn get_container_info(container_name: &str) -> Self {
        let status_output = Command::new("docker")
            .args(&["inspect", container_name, "--format", "{{.State.Status}}"])
            .output()
            .expect("msg");

        let container_status: String = from_utf8(&status_output.stdout).unwrap().trim().to_string();

        if container_status != "running" {
            return Self {
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

        let cpu_percent = cpu_str.trim_end_matches("%").parse::<usize>().unwrap_or(0);
        let memory_percent = mem_perc_str
            .trim_end_matches("%")
            .parse::<usize>()
            .unwrap_or(0);
        let memory_usage = mem_str;

        let status = Self::get_health_status(container_name);

        Self {
            name: container_name.to_string(),
            status,
            container_status,
            cpu_percent,
            memory_usage,
            memory_percent,
        }
    }

    fn get_health_status(container_name: &str) -> HealthStatus {
        HealthStatus::Healthy
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // for _ in 0..args.count {
    //     println!("Hello {}!", args.name);
    // }

    // let container_names = cli.name.as_deref().unwrap();
    println!("🐳 Welcome to Docker Container Health Monitor!");

    let container_names = match cli.name {
        Some(names) if !names.is_empty() => names,
        _ => {
            eprintln!("No container names provided. exiting.");
            return Ok(());
        }
    };

    // database setup
    let sqlite = sqlite::open("/home/fahdashour/container-health-monitor/db/monitor.db").unwrap();
    let query = "
        create table if not exists containers (id text unique, name text unique, container_status text, health text,);
        ";

    sqlite.execute(query).unwrap();
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
            let add_containers_query = "
            insert into containers values (?,?, 'running') returning *;
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
            let container = ContainerHealth {
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

    container_stats("redis")?;
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
    let cmd = binding.args(&["stats", "--no-stream", "--format"]);
    let cpu_str = from_utf8(&cmd.args(["{{.CPUPerc}}"]).output()?.stdout)?
        .trim()
        .to_owned();
    let mem_perc_str = from_utf8(&cmd.args(["{{.MemPerc}}"]).output()?.stdout)?
        .trim()
        .to_owned();
    let mem_str = from_utf8(&cmd.args(["{{.MemUsage}}"]).output()?.stdout)?
        .trim()
        .to_owned();

    let cpu_percent = cpu_str.trim_end_matches("%").parse::<usize>().unwrap_or(0);
    let memory_percent = mem_perc_str
        .trim_end_matches("%")
        .parse::<usize>()
        .unwrap_or(0);
    let memory_usage = mem_str;

    Ok(ContainerHealth {
        name: container_name.to_string(),
        status: HealthStatus::Healthy,
        container_status,
        cpu_percent,
        memory_usage,
        memory_percent,
    })
}

fn get_container_status(container_name: &str) -> String {
    let status_output = Command::new("docker")
        .args(&["inspect", container_name, "--format", "{{.State.Status}}"])
        .output()
        .expect("msg");

    from_utf8(&status_output.stdout).unwrap().trim().to_string()
}
