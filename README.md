# Foodly Backend

This is the backend server for **Foodly**, a collaborative recipe management application.
The backend handles real-time live editing, offline synchronization, and shared recipes using WebSockets and a custom
change backlog strategy.

> **Frontend**: The companion mobile and web frontend for Foodly can be found
> here: [Foodly-FE](https://github.com/P1umPudding/Foodly-FE)

## Features

- **Real-Time Live Editing**: Edit recipes collaboratively with other users in real-time via WebSockets.
- **Offline Sync**: Client changes are cached offline as a change backlog and synchronized when the connection is
  restored, automatically resolving conflicts.
- **Shared Recipes**: Full support for sharing recipes with specific permissions (owners, editors, and viewers).
- **Structured Data**: Supports complex recipe elements including sections, ingredients, tags, images, and user ratings.

## Tech Stack

The backend is built in Rust to ensure high performance, memory safety, and robust concurrency handling.

- **Language**: Rust
- **Web Framework**: [Axum](https://github.com/tokio-rs/axum) (HTTP & WebSockets)
- **Database**: PostgreSQL
- **Database Driver**: [SQLx](https://github.com/launchbadge/sqlx) (Async, compile-time checked SQL)
- **Authentication**: JWT via REST endpoint
- **Async Runtime**: Tokio

See the [implementation plan](resources/md/IMPLEMENTATION_PLAN.md) for more details on the architecture and design
decisions.

## Architecture

- **WebSocket API**: Primary communication layer for syncing recipe changes and broadcasting updates to subscribed
  clients.
- **REST API**: Handles initial connection bootstrapping, such as authentication (`/login`).
- **Sync Strategy**: Custom backlog processing. The server applies offline changes sequentially and drops incoming
  conflicting changes to maintain a consistent state.

## Installation & Setup

To get a local development environment running with mock data, follow these steps:

1. **Start the database:**
   We use Docker Compose to spin up a local PostgreSQL instance.
   ```bash
   docker compose up -d
   ```
2. **Setup Tables:**
   `cd ./src/db/migrations` an then run the follwing inside this directory (creates the tables) `for file in ./*; do [ -f "$file" ] && docker exec -i foodly_postgres psql -U postgres -d foodly < $file; done`
   This is usally done by the server directly but due to the structure of sqlx you cant do a `cargo run` because it won't compile without the required tables.

3. **Seed the database (Optional):**
   If you want to populate the database with mock recipes and users to start testing immediately, you can apply the provided seed script directly into the running Postgres container:
   ```bash
   docker exec -i foodly_postgres psql -U postgres -d foodly < resources/mock/seed.sql
   ```

4. **Run the server:**
   Ensure you have configured `.env` correctly (a `.env.example` is provided), then run the application:
   ```bash
   cargo run
   ```

## License

See the [LICENSE](LICENSE) file for more information.
