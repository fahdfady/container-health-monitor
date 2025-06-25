use std::process::Command;

use color_print::cprintln;
use redis::{self, Client, Commands};

fn main() -> redis::RedisResult<()> {
    println!("Hello, world!");

    cprintln!("connecting to redis..");
    let redis_client = Client::open("redis://127.0.0.1/")?;
    let mut conn = redis_client.get_connection()?;
    cprintln!("<green>Redis Server Connected</green>");

    let _: () = conn.set_ex("key", "value", 1)?;

    let container_status = get_container_status("sad_pare");
    println!("container status {}", container_status);
    Ok(())
}

fn get_container_status(name: &str) -> String {
    let output = Command::new("docker")
        .args(&["inspect", name, "--format", "{{.State.Status}}"])
        .output()
        .expect("msg");

    std::str::from_utf8(&output.stdout)
        .unwrap()
        .trim()
        .to_string()
}
