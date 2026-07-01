# 02 - API Design

The API will be primarily a versioned REST API (`/api/v1/...`), with WebSockets integrated for real-time features (live editing) in a future phase.

## REST Endpoints (v1)

### Authentication & Users
- `POST /api/v1/auth/login` - Authenticate and receive a JWT. (JWT-based access/refresh tokens. Social logins are not required.)
- `POST /api/v1/auth/register` - Create a new user account.
- `POST /api/v1/auth/ws-ticket` - Generate a short-lived (10-30s) one-time ticket for WebSocket authentication. Requires standard JWT auth.
- `GET /api/v1/users/me` - Get current user profile.

### Recipes
- `GET /api/v1/recipes` - List recipes (uses cursor-based pagination, returns preview data).
- `GET /api/v1/recipes/:id` - Get a single recipe (returns full data).
- `POST /api/v1/recipes` - Create a new recipe.
- `PUT /api/v1/recipes/:id` - Update a recipe (full or partial).
- `DELETE /api/v1/recipes/:id` - Delete a recipe.

### Taxonomy & Metadata
- `GET /api/v1/ingredients` - List all ingredients (catalog).
- `GET /api/v1/tags` - List all tags.

### User Organization
- `GET /api/v1/categories` - List user categories.
- `POST /api/v1/categories` - Create/manage categories.
- `GET /api/v1/groups` - List user groups.

### Media
- `POST /api/v1/images` - Upload an image (returns an Image ID and Hash).
- `GET /api/v1/images/:hash` - Retrieve an image.

## Real-time & Offline Sync

For the initial phase, offline backlog processing will be handled via a standard REST endpoint to keep the architecture simple. WebSockets (WSS) will be introduced in a future phase specifically for live editing.

- `POST /api/v1/sync` - Submit an offline change backlog array and receive a resolution response.
