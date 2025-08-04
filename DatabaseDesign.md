# Database Design
This document outlines the schema for the SQLite database used in this project.

## Table: `containers`

Stores the latest state of monitored Docker containers.

| Column Name      | Type    | Description                          |
|------------------|---------|--------------------------------------|
| `id`             | TEXT    | Docker container ID (hash)           |
| `name`           | TEXT    | container name                       |
| `container_state`| TEXT    | Docker state (e.g., running, exited) |
| `status`         | TEXT    | Health or custom status              |
| `last_updated`   | DATETIME| Timestamp for cache invalidation     |


## Table: `container_history`

Captures historical snapshots of container data history for stats and analytics, to observe the container info around the time.

| Column Name      | Type    | Description                          |
|------------------|---------|--------------------------------------|
| `id`             | TEXT    | Docker container ID (hash)           |
| `name`           | TEXT    | Name at the time of the snapshot     |
| `container_state`| TEXT    | State during the snapshot            |
| `status`         | TEXT    | Status during the snapshot           |
| `cpu_percent`    | REAL    | CPU usage percentage                 |
| `memory_percent` | REAL    | Memory usage percentage              |
| `restart_count`  | TEXT    | restart count during the snapshot    |
| `uptime`         | TEXT    | Human-readable uptime                |
| `timestamp`      | INTEGER | Unix timestamp of snapshot           |


## Notes
- `container_history` enables trends and uptime/downtime analytics.

## Decisions are based on:

Store in History if:
- Changes frequently (CPU, memory change every second)
- Trends matter (detecting memory leaks, CPU spikes)

Store in Current State if:
- Need fast lookups (dashboard showing current "status")
- Composite derived data (health status calculated from metrics)
- Cache invalidation (when was this last refreshed?)