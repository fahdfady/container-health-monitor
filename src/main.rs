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

    Ok(())
}

fn get_container_status(name: &str) -> String {
    let status_output = Command::new("docker")
        .args(&["inspect", name, "--format", "{{.State.Status}}"])
        .output()
        .expect("msg");

    std::str::from_utf8(&status_output.stdout)
        .unwrap()
        .trim()
        .to_string()
}
