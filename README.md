# Docker Container Health Monitor (chm)

## Prerequisites

- **Docker**: Make sure you have Docker running on your machine
- **Rust**: To build the binary
- **Sqlite**: Required for storage
- **Redis**: A running Redis instance

## Installation

1. Clone the repository
```bash
git clone https://github.com/fahdfady/cotainer-health-monitor
cd container-health-monitor
```

2. Build the project using Cargo
```bash
cargo build --release
```

3. you now have the binary as `chm`
```bash
cd target/release/
./chm
```

## Usage

### command structure

- from cargo:
```bash
cargo run -- [COMMAND]
```

- from the binary
```bash
./chm [COMMAND]
```

### commands

1. `monitor`
Monitor specific containers by their names.

Options:
- `-n, --name <NAME>`: Specify the name for one or more container names to monitor (required).
- `-c, --cache-ttl <SECONDS>`: Set the Redis cache TTL in seconds (default: 60).
- `-w, --watch`: Enable watch mode to continuously monitor containers (default: false).

Example:
```bash
cargo run -- monitor --name container1 --name container2 --cache-ttl 120 --watch
```
This monitors container1 and container2, caching data for 120 seconds, and continuously updates in watch mode.


2. `monitor-all`
Monitor all containers running on the machine.

Options:
`-c, --cache-ttl <SECONDS>`: Set the Redis cache TTL in seconds (default: 60).
`-w, --watch`: Enable watch mode to continuously monitor containers (default: false).

Example:
```bash
cargo run -- monitor-all --cache-ttl 120 --watch
```
This monitors all containers on the machine, caching data for 120 seconds, and continuously updates in watch mode.


3. `wipe`
Wipe and Delete all data you have on the database. and start clean with a new history

Example:
```bash
cargo run -- wipe
```

## Contributions
To contribute, check the [`todo.md`](todo.md) file or [issues](https://github.com/fahdfady/container-health-monitor/issues) for outstanding tasks and submit pull requests.

## License
This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.