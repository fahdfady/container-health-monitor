use std::fmt;
use std::process::Command;

use color_print::cprintln;
use redis::{self, Client, Commands};

struct ContainerHealth {
    name: String,
    status: String,
    cpu_percent: usize,
    memory_usage: String,
    memory_percent: usize,
}

impl fmt::Display for ContainerHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let status_emoji: &str = match self.status.as_str() {
            "running" => "🟢",
            "exited" => "🔴",
            _ => "⚪",
        };

        write!(
            f,
            "{} {} {} | CPU: {:.1}% | Mem: {} ({:.1}%)",
            status_emoji,
            self.name,
            self.status,
            self.cpu_percent,
            self.memory_usage,
            self.memory_percent
        )
    }
}

impl ContainerHealth {
    fn health_data(&self) -> String {
        // ? should this return &str instead?
        format!(
            "{{name:{}, status:{}, cpu_percentage:{}, memory_usage:{}, memory_percentage:{}, snapshot_took_at:{}}}",
            self.name,
            self.status,
            self.cpu_percent,
            self.memory_usage,
            self.memory_percent,
            chrono::Utc::now().to_rfc3339()
        )
    }
}

fn main() -> redis::RedisResult<()> {
    println!("🐳 Welcome to Docker Container Health Monitor!");

    cprintln!("connecting to redis..");
    let redis_client = Client::open("redis://127.0.0.1/")?;
    let mut conn = redis_client.get_connection()?;
    cprintln!("<green>Redis Server Connected</green>");
    let _: () = conn.set("health_monitor:status", true)?;

    let container = ContainerHealth {
        name: String::from("sad_pare"),
        status: String::from("running"),
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
    let status = get_container_status(container_name);

    let mut binding = Command::new("docker");
    let cmd = binding.args(&["stats", "--no-stream", "format"]);
    let cpu_str = if status == "running" {
        std::str::from_utf8(&cmd.args(["{{.CPUPerc}}"]).output()?.stdout)?.trim().to_owned()
    } else {
        "0%".to_string()
    };

    let cpu_percent = cpu_str.trim_end_matches("%").parse::<usize>().unwrap_or(0);

    println!("CPU PERCENT {}", cpu_percent);
    let memory_percent: usize = if status == "running" { 2 } else { 0 };
    let memory_usage: String = if status == "running" {
        String::from("eqweqwe")
    } else {
        String::from("")
    };

    Ok(ContainerHealth {
        name: container_name.to_string(),
        status,
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
