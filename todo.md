- [x] add a cli utility to enter the data of a container (name)
- [x] use sqlite for storing

- [x] monitor all containers on the machine
- [x] watch a container for changes
- [ ] montior containers concurrently
- [ ] feat: make an alert system
    - eg. alert if container restated via daemon 3 times
- [x] format container health data to something like tables
- [ ] add a container history table in sqlite
    - this will help us track the history of tables, not just the current state, to make dashboards and insights of our containers
- [ ] utilize redis caching to retrive current info of a container faster
- [ ] consider using [bollard](https://docs.rs/bollard/latest/bollard/)

- [ ] special requests via cli
    - [x] Add a `wipe` CLI subcommand For devs or power users who want to start from a clean DB.


PERF:
- noticed that stopping a container (making the state=Exited) results in making the data printing in CLI faster. this means we have a performance issue here. I think this is because of the time used in executing the `docker` command in CLI, this also is highly noticed when running monitoring on multiple containers. maybe using bollard (which deals with docker daemon directly) will solve this problem or at least make the performance much better and faster.