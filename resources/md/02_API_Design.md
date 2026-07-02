<# 02 - API Design

The API is a versioned REST API (`/api/v1/...`). WebSockets will be integrated in
a future phase for real-time live editing.

All endpoints (except login and register) require a valid JWT in the
`Authorization: Bearer <token>` header. Responses use standard HTTP status codes
and a consistent JSON error envelope.

---

## Conventions

### Naming

- Plural nouns for resource collections: `/recipes`, `/tags`, `/users`.
- Singular sub-resources via `:id`: `/recipes/:id`.
- Actions that don't map to CRUD use a verb suffix: `/auth/login`, `/auth/refresh`, `/recipes/:id/copy`.
- Nested ownership is expressed through URL nesting only where the parent is needed for scoping (e.g. `/recipes/:id/ratings`). Globally unique resources stay at the top level.

### Pagination

List endpoints that can grow unbounded use **cursor-based pagination**:

```
GET /api/v1/recipes?cursor=<opaque>&limit=20
```

| Parameter | Type             | Default | Description                                       |
|-----------|------------------|---------|---------------------------------------------------|
| `cursor`  | `string \| null` | `null`  | Opaque cursor from the previous response. Omit for the first page. |
| `limit`   | `integer`        | `20`    | Max items per page (server caps at 100).          |

Response wrapper:

```json
{
  "data": [ ... ],
  "cursor": "eyJpZCI6NDJ9"   // null when no more pages
}
```

Small, bounded catalogs (`/tags`, `/ingredients`) return a flat array (no pagination).

### Error Envelope

All error responses use:

```json
{
  "error": {
    "code": "RECIPE_NOT_FOUND",
    "message": "Recipe with id 42 does not exist."
  }
}
```

Standard status codes used: `400` (validation), `401` (unauthenticated), `403` (forbidden), `404` (not found), `409` (conflict), `422` (unprocessable), `500` (internal).

### Timestamps

All timestamps are ISO 8601 UTC strings (`"2026-01-15T14:30:00Z"`).

---

## Authentication & Account

### `POST /api/v1/auth/register`

Create a new user account.

**Request:**
```json
{
  "name": "Kolja",
  "email": "kolja@example.com",
  "password": "hunter2"
}
```

**Response `201 Created`:**
```json
{
  "user": {
    "id": 1,
    "name": "Kolja",
    "email": "kolja@example.com",
    "profilePicture": null
  },
  "accessToken": "eyJ...",
  "refreshToken": "dGhpcyBpcyBhIHJlZnJlc2g..."
}
```

**Errors:**
- `409 Conflict` — email already registered.
- `422 Unprocessable` — validation failure (missing fields, weak password, etc.).

---

### `POST /api/v1/auth/login`

Authenticate with credentials and receive a JWT token pair.

**Request:**
```json
{
  "email": "kolja@example.com",
  "password": "hunter2"
}
```

**Response `200 OK`:**
```json
{
  "user": {
    "id": 1,
    "name": "Kolja",
    "email": "kolja@example.com",
    "profilePicture": null
  },
  "accessToken": "eyJ...",
  "refreshToken": "dGhpcyBpcyBhIHJlZnJlc2g..."
}
```

**Errors:**
- `401 Unauthorized` — invalid email or password.

---

### `POST /api/v1/auth/refresh`

Exchange a valid refresh token for a new access/refresh token pair.

**Request:**
```json
{
  "refreshToken": "dGhpcyBpcyBhIHJlZnJlc2g..."
}
```

**Response `200 OK`:**
```json
{
  "accessToken": "eyJ...(new)...",
  "refreshToken": "ZnJlc2gtcmVmcmVzaA..."
}
```

**Errors:**
- `401 Unauthorized` — refresh token expired or revoked.

---

### `POST /api/v1/auth/ws-ticket`

Generate a short-lived (10–30 s) one-time ticket for authenticating a WebSocket connection. This prevents long-lived JWTs from appearing in query parameters or reverse-proxy logs.

**Request:** *(empty body; auth via JWT header)*

**Response `201 Created`:**
```json
{
  "ticket": "a1b2c3d4-e5f6-...",
  "expiresIn": 30
}
```

---

## Current User

### `GET /api/v1/users/me`

Return the authenticated user's own profile. Used by the frontend `CatalogProvider` on startup to identify the current user.

**Response `200 OK`:**
```json
{
  "id": 1,
  "name": "Kolja",
  "email": "kolja@example.com",
  "profilePicture": "a3f8c2..."
}
```

---

### `PATCH /api/v1/users/me`

Update the authenticated user's profile fields. Only provided fields are updated.

**Request:**
```json
{
  "name": "Kolja M."
}
```

**Response `200 OK`:** *(returns the full updated user object, same shape as `GET /users/me`)*

**Errors:**
- `422 Unprocessable` — name empty, etc.

---

### `PUT /api/v1/users/me/profile-picture`

Upload or replace the user's profile picture. Body is the raw image binary (`Content-Type: image/jpeg`, `image/png`, or `image/webp`).

**Response `200 OK`:**
```json
{
  "profilePicture": "b7e2d1..."
}
```

---

### `DELETE /api/v1/users/me/profile-picture`

Remove the user's profile picture.

**Response `204 No Content`**

---

## User Search (for invitations)

### `GET /api/v1/users/search`

Search for users by name (or partial name) for use in "invite to recipe" or "add to group" dropdowns. Returns a compact list of matching users. **Does not return the full user object** — only the fields needed for the dropdown display.

**Query parameters:**

| Parameter | Type     | Required | Description                                                  |
|-----------|----------|----------|--------------------------------------------------------------|
| `q`       | `string` | yes      | Search query (min 1 char). Matches against user `name` (case-insensitive, prefix/substring). |
| `limit`   | `integer`| no       | Max results (default `10`, server cap `50`).                 |

**Response `200 OK`:**
```json
{
  "data": [
    { "id": 2, "name": "Mara", "profilePicture": null },
    { "id": 3, "name": "Jonas", "profilePicture": "c4f1..." }
  ]
}
```

> The authenticated user (the one performing the search) is **excluded** from results — you don't invite yourself.

**Errors:**
- `400 Bad Request` — `q` is missing or empty.

---

## Recipes

### `GET /api/v1/recipes`

List recipes accessible to the authenticated user (owned + shared as editor/viewer). Returns **preview data** — a subset of fields optimized for the list view. Sections, ingredients, steps, and notes are **not** included.

**Query parameters:** `cursor`, `limit` (see Pagination above).

**Response `200 OK`:**
```json
{
  "data": [
    {
      "id": 1,
      "owner": 1,
      "editors": [2],
      "viewers": [3],
      "name": "Spaghetti Bolognese",
      "tags": ["Hauptgericht", "Italienisch"],
      "source": "https://example.com/bolognese",
      "rating": [
        { "user": 1, "rating": 5 },
        { "user": 2, "rating": 4 }
      ],
      "time": "45 min",
      "workMinutes": 20,
      "overallMinutes": 45,
      "sizeNumber": 4,
      "sizeText": "{Portionen}",
      "mainImage": 1,
      "createdAt": "2026-01-10T12:00:00Z",
      "updatedAt": "2026-03-15T09:30:00Z"
    }
  ],
  "cursor": "eyJpZCI6MX0"
}
```

> **Scope:** The backend **only returns recipes where the user is `owner`, `editor`, or `viewer`**. The frontend does not need to filter by access itself.

---

### `GET /api/v1/recipes/:id`

Get a single recipe with **full data** including sections, ingredients, steps, notes, and images.

**Response `200 OK`:**
```json
{
  "id": 1,
  "owner": 1,
  "editors": [2],
  "viewers": [3],
  "name": "Spaghetti Bolognese",
  "tags": ["Hauptgericht", "Italienisch"],
  "source": "https://example.com/bolognese",
  "rating": [
    { "user": 1, "rating": 5 },
    { "user": 2, "rating": 4 }
  ],
  "time": "45 min",
  "workMinutes": 20,
  "overallMinutes": 45,
  "sizeNumber": 4,
  "sizeText": "{Portionen}",
  "notes": ["Am besten mit frischem Basilikum servieren."],
  "mainImage": 1,
  "images": [2, 3],
  "sections": [
    {
      "id": 1,
      "name": null,
      "ingredients": [
        {
          "id": 1,
          "ingredient": { "id": 1, "name": "Spaghetti" },
          "text": null,
          "amount": "500",
          "amountPrefix": null,
          "unit": "g"
        },
        {
          "id": 2,
          "ingredient": null,
          "text": "etwas Petersilie zum Garnieren",
          "amount": null,
          "amountPrefix": null,
          "unit": null
        }
      ],
      "steps": [
        "Nudeln in Salzwasser kochen.",
        "Hackfleisch anbraten."
      ]
    }
  ],
  "createdAt": "2026-01-10T12:00:00Z",
  "updatedAt": "2026-03-15T09:30:00Z"
}
```

**Errors:**
- `404 Not Found` — recipe does not exist.
- `403 Forbidden` — user is not owner/editor/viewer.

---

### `POST /api/v1/recipes`

Create a new recipe. The authenticated user becomes the `owner`.

**Request:**
```json
{
  "name": "Neues Rezept",
  "tags": ["Hauptgericht"],
  "source": null,
  "time": null,
  "workMinutes": null,
  "overallMinutes": null,
  "sizeNumber": 4,
  "sizeText": "{Portionen}",
  "notes": [],
  "mainImage": null,
  "images": [],
  "sections": [
    {
      "name": null,
      "ingredients": [
        {
          "ingredient": 1,
          "text": null,
          "amount": "200",
          "amountPrefix": null,
          "unit": "g"
        }
      ],
      "steps": ["Schritt 1"]
    }
  ]
}
```

> Note: In the create/update request, `ingredient` is just the `IngredientId` (a number) or `null`. The server expands it to `{ id, name }` in responses.

**Response `201 Created`:** *(full recipe object, same shape as `GET /recipes/:id`)*

**Errors:**
- `422 Unprocessable` — validation failure (empty name, etc.).

---

### `PUT /api/v1/recipes/:id`

Full update of a recipe. Replaces the entire recipe content. Requires `owner` or `editor` role.

**Request:** *(same shape as `POST /recipes`, all fields required)*

**Response `200 OK`:** *(full recipe object)*

**Errors:**
- `403 Forbidden` — user is viewer or has no access.
- `404 Not Found` — recipe does not exist.

---

### `DELETE /api/v1/recipes/:id`

Delete a recipe. Requires `owner` role.

**Response `204 No Content`**

**Errors:**
- `403 Forbidden` — user is not owner.
- `404 Not Found` — recipe does not exist.

---

### `POST /api/v1/recipes/:id/copy`

Create a personal copy of a recipe. The authenticated user becomes the owner of the copy; `editors` and `viewers` are empty on the new copy. Requires at least `viewer` access on the source recipe.

**Request:** *(empty body)*

**Response `201 Created`:** *(full recipe object of the new copy)*

---

## Recipe Sharing

Managing who can view/edit a recipe. Only the `owner` may change permissions.

### `PUT /api/v1/recipes/:id/editors`

Set the editor list for a recipe (replaces the current list).

**Request:**
```json
{
  "userIds": [2, 5]
}
```

**Response `200 OK`:**
```json
{
  "editors": [2, 5]
}
```

**Errors:**
- `403 Forbidden` — user is not owner.

---

### `PUT /api/v1/recipes/:id/viewers`

Set the viewer list for a recipe (replaces the current list).

**Request:**
```json
{
  "userIds": [3, 7]
}
```

**Response `200 OK`:**
```json
{
  "viewers": [3, 7]
}
```

**Errors:**
- `403 Forbidden` — user is not owner.

---

## Ratings

Ratings are embedded inside the recipe object on read. These endpoints manage the current user's own rating on a recipe.

### `PUT /api/v1/recipes/:id/ratings/me`

Set or update the current user's rating for a recipe. Requires at least `viewer` access.

**Request:**
```json
{
  "rating": 4
}
```

> `rating` is an integer 1–5.

**Response `200 OK`:**
```json
{
  "user": 1,
  "rating": 4
}
```

---

### `DELETE /api/v1/recipes/:id/ratings/me`

Remove the current user's rating from a recipe.

**Response `204 No Content`**

---

## User Categories

Categories are **per-user** — each user has their own set. The authenticated user can only access their own categories.

### `GET /api/v1/categories`

List all categories of the authenticated user, ordered by `order`.

**Response `200 OK`:**
```json
{
  "data": [
    {
      "id": 1,
      "user": 1,
      "name": "Favoriten",
      "recipes": [1, 6, 8, 13],
      "order": 0,
      "color": "#e11d48",
      "colorLight": null,
      "colorDark": null
    }
  ]
}
```

---

### `POST /api/v1/categories`

Create a new category.

**Request:**
```json
{
  "name": "Schnelle Küche",
  "color": "#0ea5e9",
  "colorLight": null,
  "colorDark": null
}
```

**Response `201 Created`:** *(full category object; `recipes` starts as `[]`, `order` is auto-assigned)*

---

### `PUT /api/v1/categories/:id`

Update a category's metadata (name, colors). Does **not** change the recipe list — use the recipe-assignment endpoints below.

**Request:**
```json
{
  "name": "Favoriten ⭐",
  "color": "#dc2626",
  "colorLight": "#fecaca",
  "colorDark": "#991b1b"
}
```

**Response `200 OK`:** *(full category object)*

**Errors:**
- `404 Not Found` — category does not exist or belongs to another user.

---

### `DELETE /api/v1/categories/:id`

Delete a category. Recipes in it are **not** deleted; they become uncategorized.

**Response `204 No Content`**

---

### `PUT /api/v1/categories/:id/recipes`

Set the ordered list of recipe IDs in this category (replaces the current list).

**Request:**
```json
{
  "recipeIds": [1, 8, 6, 13]
}
```

**Response `200 OK`:**
```json
{
  "recipes": [1, 8, 6, 13]
}
```

**Errors:**
- `422 Unprocessable` — recipe IDs that the user doesn't have access to.

---

### `PUT /api/v1/categories/order`

Reorder all categories at once. Expects the full list of category IDs in the desired order.

**Request:**
```json
{
  "categoryIds": [3, 1, 2, 4]
}
```

**Response `200 OK`:**
```json
{
  "data": [
    { "id": 3, "order": 0 },
    { "id": 1, "order": 1 },
    { "id": 2, "order": 2 },
    { "id": 4, "order": 3 }
  ]
}
```

---

## Groups

Groups are user-curated sets of users, used for quick bulk-assignment when sharing recipes. A group has a single `owner` (its creator).

### `GET /api/v1/groups`

List all groups where the authenticated user is `owner` or `member`.

**Response `200 OK`:**
```json
{
  "data": [
    {
      "id": 1,
      "name": "Familie",
      "owner": 1,
      "members": [2, 3]
    }
  ]
}
```

---

### `POST /api/v1/groups`

Create a new group. The authenticated user becomes the `owner`.

**Request:**
```json
{
  "name": "Familie",
  "members": [2, 3]
}
```

**Response `201 Created`:** *(full group object)*

---

### `PUT /api/v1/groups/:id`

Update a group (name and/or members). Requires `owner` role.

**Request:**
```json
{
  "name": "Familie & Freunde",
  "members": [2, 3, 5]
}
```

**Response `200 OK`:** *(full group object)*

**Errors:**
- `403 Forbidden` — user is not group owner.

---

### `DELETE /api/v1/groups/:id`

Delete a group. Does **not** affect recipe access — users already added to recipes via this group keep their permissions.

**Response `204 No Content`**

**Errors:**
- `403 Forbidden` — user is not group owner.

---

## Taxonomy & Metadata (Catalogs)

Global, read-only catalogs. The frontend loads these once at startup (`CatalogProvider`). They are not paginated (bounded in size).

### `GET /api/v1/tags`

List all tags.

**Response `200 OK`:**
```json
{
  "data": [
    { "id": "Hauptgericht", "svg": null },
    { "id": "Vegan", "svg": "vegan" }
  ]
}
```

> `tag.id` **is** the display name (e.g. `"Hauptgericht"`). `svg` is a hash/filename of an optional icon SVG, or `null`.

---

### `GET /api/v1/ingredients`

List the full ingredient catalog. Users cannot add to this — it's curated server-side.

**Response `200 OK`:**
```json
{
  "data": [
    { "id": 1, "name": "Spaghetti" },
    { "id": 2, "name": "Hackfleisch (Rind)" }
  ]
}
```

---

## Media

Images are stored on the server's disk, addressed by content hash. The database stores only the hash (which doubles as the filename). Files are stored in a nested folder structure (e.g. `images/ab/cd/abcdef...`) to avoid directory overload.

### `POST /api/v1/images`

Upload an image. Body is the raw image binary (`Content-Type: image/jpeg`, `image/png`, or `image/webp`).

**Response `201 Created`:**
```json
{
  "id": 7,
  "hash": "a3f8c2d1e5...",
  "name": null
}
```

---

### `GET /api/v1/images/:hash`

Retrieve an image by its content hash. Returns the raw image binary with the appropriate `Content-Type` header. Supports `If-None-Match` / `ETag` for caching (the hash *is* the ETag).

**Response `200 OK`:** *(binary image data)*

**Headers:**
```
Content-Type: image/jpeg
ETag: "a3f8c2d1e5..."
Cache-Control: public, max-age=31536000, immutable
```

> Because images are content-addressed (hash = filename), they are immutable — aggressive caching is safe.

**Errors:**
- `404 Not Found` — no image with this hash.

---

### `GET /api/v1/tags/:id/icon`

Retrieve a tag's SVG icon by tag ID. Returns the raw SVG. Only available for tags where `svg` is non-null.

**Response `200 OK`:** *(raw SVG data)*

**Headers:**
```
Content-Type: image/svg+xml
Cache-Control: public, max-age=86400
```

**Errors:**
- `404 Not Found` — tag has no icon.

---

## Offline Sync

For the initial phase, offline backlog processing is handled via a REST endpoint. WebSockets (WSS) will be introduced in a later phase for live editing.

### `POST /api/v1/sync`

Submit an offline change backlog. The server processes changes sequentially, applying valid ones and dropping conflicting ones.

**Request:**
```json
{
  "changes": [
    {
      "type": "recipe.update",
      "recipeId": 1,
      "timestamp": "2026-06-28T10:00:00Z",
      "data": { "name": "Spaghetti Bolognese (updated)" }
    },
    {
      "type": "recipe.create",
      "tempId": "tmp-abc-123",
      "timestamp": "2026-06-28T10:01:00Z",
      "data": { "name": "Neues Rezept", "...": "..." }
    }
  ]
}
```

> `tempId` is a client-generated temporary ID for entities created offline. The server maps these to real server-assigned IDs in the response.

**Response `200 OK`:**
```json
{
  "applied": [
    { "index": 0, "status": "applied" },
    { "index": 1, "status": "applied", "serverId": 42, "tempId": "tmp-abc-123" }
  ],
  "dropped": [
    { "index": 2, "reason": "conflict", "message": "Recipe was deleted by another user." }
  ]
}
```

---

## WebSocket Protocol (Future Phase)

Connection: `wss://<host>/api/v1/ws?ticket=<one-time-ticket>`

### Message Envelope

```
client → server (request):  { id: string, type: string, payload?: unknown }
server → client (response): { id: string, ok: boolean, result?: unknown, error?: string }
server → client (event):    { type: string, payload?: unknown }   // no id
```

### Planned Message Types

| Type               | Direction      | Description                                              |
|--------------------|----------------|----------------------------------------------------------|
| `me`               | request        | Get current user profile                                 |
| `recipes.list`     | request        | List accessible recipes (preview data)                   |
| `recipes.get`      | request        | Get full recipe by ID                                    |
| `categories.list`  | request        | List user's categories                                   |
| `tags.list`        | request        | List all tags                                            |
| `ingredients.list` | request        | List ingredient catalog                                  |
| `users.list`       | request        | List all users (for catalog/invite dropdown, deprecated in favor of `users.search`) |
| `users.search`     | request        | Search users by name for invite dropdowns                |
| `recipe.updated`   | server event   | A recipe the client is subscribed to was changed         |
| `recipe.deleted`   | server event   | A recipe was deleted                                     |
| `sync.submit`      | request        | Submit offline change backlog                            |

---

## Endpoint Summary

| Method   | Path                                  | Auth | Description                                      |
|----------|---------------------------------------|------|--------------------------------------------------|
| `POST`   | `/api/v1/auth/register`               | no   | Create account                                   |
| `POST`   | `/api/v1/auth/login`                  | no   | Login, get tokens                                |
| `POST`   | `/api/v1/auth/refresh`                | no   | Refresh access token                             |
| `POST`   | `/api/v1/auth/ws-ticket`              | yes  | Generate one-time WS ticket                      |
| `GET`    | `/api/v1/users/me`                    | yes  | Current user profile                             |
| `PATCH`  | `/api/v1/users/me`                    | yes  | Update own profile                               |
| `PUT`    | `/api/v1/users/me/profile-picture`    | yes  | Upload/replace profile picture                   |
| `DELETE` | `/api/v1/users/me/profile-picture`    | yes  | Remove profile picture                           |
| `GET`    | `/api/v1/users/search?q=...`          | yes  | Search users by name (invite dropdown)           |
| `GET`    | `/api/v1/recipes`                     | yes  | List recipes (preview, paginated)                |
| `GET`    | `/api/v1/recipes/:id`                 | yes  | Get full recipe                                  |
| `POST`   | `/api/v1/recipes`                     | yes  | Create recipe                                    |
| `PUT`    | `/api/v1/recipes/:id`                 | yes  | Update recipe                                    |
| `DELETE` | `/api/v1/recipes/:id`                 | yes  | Delete recipe                                    |
| `POST`   | `/api/v1/recipes/:id/copy`            | yes  | Copy recipe to own                               |
| `PUT`    | `/api/v1/recipes/:id/editors`         | yes  | Set editor list                                  |
| `PUT`    | `/api/v1/recipes/:id/viewers`         | yes  | Set viewer list                                  |
| `PUT`    | `/api/v1/recipes/:id/ratings/me`      | yes  | Set own rating                                   |
| `DELETE` | `/api/v1/recipes/:id/ratings/me`      | yes  | Remove own rating                                |
| `GET`    | `/api/v1/categories`                  | yes  | List own categories                              |
| `POST`   | `/api/v1/categories`                  | yes  | Create category                                  |
| `PUT`    | `/api/v1/categories/:id`              | yes  | Update category metadata                         |
| `DELETE` | `/api/v1/categories/:id`              | yes  | Delete category                                  |
| `PUT`    | `/api/v1/categories/:id/recipes`      | yes  | Set recipes in category                          |
| `PUT`    | `/api/v1/categories/order`            | yes  | Reorder categories                               |
| `GET`    | `/api/v1/groups`                      | yes  | List own groups                                  |
| `POST`   | `/api/v1/groups`                      | yes  | Create group                                     |
| `PUT`    | `/api/v1/groups/:id`                  | yes  | Update group                                     |
| `DELETE` | `/api/v1/groups/:id`                  | yes  | Delete group                                     |
| `GET`    | `/api/v1/tags`                        | yes  | List all tags                                    |
| `GET`    | `/api/v1/tags/:id/icon`               | yes  | Get tag SVG icon                                 |
| `GET`    | `/api/v1/ingredients`                 | yes  | List ingredient catalog                          |
| `POST`   | `/api/v1/images`                      | yes  | Upload image                                     |
| `GET`    | `/api/v1/images/:hash`                | yes  | Get image by hash                                |
| `POST`   | `/api/v1/sync`                        | yes  | Submit offline change backlog                    |
