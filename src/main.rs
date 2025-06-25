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

fn main() -> redis::RedisResult<()> {
    println!("Hello, world!");

    cprintln!("connecting to redis..");
    let redis_client = Client::open("redis://127.0.0.1/")?;
    let mut conn = redis_client.get_connection()?;
    cprintln!("<green>Redis Server Connected</green>");

    let _: () = conn.set_ex("key", "value", 1)?;

    let container = ContainerHealth {
        name: String::from("sad_pare"),
        status: String::from("running"),
        cpu_percent: 10,
        memory_usage: String::from("200 MB"),
        memory_percent: 6,
    };

    let status_emoji: &str = match container.status.as_str() {
        "running" => "🟢",
        "exited" => "🔴",
        _ => "⚪",
    };

    // println!("container status: {} {}", status_emoji, container_status);

    println!(
        "{} {} {} | CPU: {:.1}% | Mem: {} ({:.1}%)",
        status_emoji,
        container.status,
        container.name,
        container.cpu_percent,
        container.memory_usage,
        container.memory_percent
    );

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
