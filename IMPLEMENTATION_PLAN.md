# Foodly Backend - Implementation Plan

This document outlines the proposed structure, core libraries, and architecture for the Foodly Rust backend.

## Core Decisions

All technical decisions have been locked in:

1. **Database Selection**: **PostgreSQL**
2. **Web Framework**: **Axum** 
3. **Sync Strategy**: **Custom Backlog processing**. The client provides an offline change backlog. The server processes it sequentially and drops any incoming conflicting changes.
4. **Authentication**: **JWT + REST Login Route**. An initial REST endpoint (`/login`) handles authentication and issues a JWT, which is then used to authenticate the WebSocket connection.
5. **Database Library**: **SQLx** (async, compile-time checked raw SQL)

## Proposed Core Libraries

Based on the requirements (Rust, WebSocket, DB, async), the following tech stack is recommended:

- **Web Framework**: `axum` (with `axum-ws` for WebSockets)
- **Async Runtime**: `tokio`
- **Database Driver**: `sqlx` (allows us to use raw SQL with compile-time checks, supports Postgres & SQLite)
- **Serialization**: `serde` and `serde_json` (crucial for TypeScript `<->` Rust type parity)
- **Error Handling**: `thiserror` (for library/internal errors) and `anyhow` (for application-level errors)
- **Logging & Telemetry**: `tracing` and `tracing-subscriber`

## Proposed Project Structure

A clean, layered architecture separating network layer (WS/API), business logic (Services), and data access (DB).

```text
foodly-backend/
├── Cargo.toml
├── src/
│   ├── main.rs            # Entry point, Tokio setup, app state initialization
│   ├── config.rs          # Environment variable loading (DB URL, ports, etc.)
│   ├── error.rs           # Global error types and HTTP/WS error mappings
│   ├── db/                # Database layer
│   │   ├── mod.rs         # Connection pool setup
│   │   └── migrations/    # SQL migration files
│   ├── models/            # Domain entities (1-to-1 mappings of database-types.ts)
│   │   ├── mod.rs
│   │   ├── recipe.rs
│   │   ├── user.rs
│   │   └── ...
│   ├── api/               # Network layer (Endpoints & WebSockets)
│   │   ├── mod.rs
│   │   ├── rest/          # Any standard HTTP routes (e.g., health check, login)
│   │   └── ws/            # WebSocket connection handlers and message routing
│   │       ├── mod.rs
│   │       ├── handler.rs
│   │       └── messages.rs # WebSocket message definitions (Incoming/Outgoing)
│   └── services/          # Business logic & Sync handling
│       ├── mod.rs
│       ├── sync.rs        # Processing the offline change backlog, resolving conflicts
│       ├── recipe.rs      # Recipe operations
│       └── user.rs        # User operations
└── resources/             # Provided TypeScript types and markdown docs
```

## Architecture Flow

1. **Client connects via WS**: The client establishes a WebSocket connection to the Rust API.
2. **Authentication**: The first message (or connection URL) contains a token. The server authenticates the user.
3. **Sync Phase**:
   - Client sends its `offline change backlog`.
   - Server validates and processes the changes (`services/sync.rs`), resolving any conflicts or dropping invalid changes.
   - Server broadcasts the accepted changes to all other connected clients that are viewing the same recipe (Live editing).
4. **Live Editing**: As users edit the recipe, messages are streamed to the server, applied to the database, and fanned out to active subscribers (`api/ws/handler.rs` -> `services/recipe.rs`).

## Next Steps

Once you answer the open questions and approve this plan, we can start setting up the `Cargo.toml`, initializing the directory structure, and defining the Rust structs based on `database-types.ts`.
