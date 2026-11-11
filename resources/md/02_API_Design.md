<# 02 - API Design

The API is a versioned REST API (`/api/v1/...`). WebSockets will be integrated in
a future phase for real-time live editing.

All endpoints require a valid JWT in the `Authorization: Bearer <token>` header,
except the public ones (`/auth/register`, `/auth/login`, `/auth/refresh`, and
`GET /images/:hash`). Responses use standard HTTP status codes and a consistent
JSON error envelope.

---

## Conventions

### Naming

- Plural nouns for resource collections: `/recipes`, `/tags`, `/users`.
- Singular sub-resources via `:id`: `/recipes/:id`.
- Actions that don't map to CRUD use a verb suffix: `/auth/login`, `/auth/refresh`, `/recipes/:id/copy`.
- Nested ownership is expressed through URL nesting only where the parent is needed for scoping (e.g. `/recipes/:id/ratings`). Globally unique resources stay at the top level.

### Pagination

List endpoints that can grow unbounded (like recipes) use **offset-based pagination** via URL query parameters:

```
?page=1&limit=20
```

| Parameter | Type             | Default | Description                                                        |
|-----------|------------------|---------|--------------------------------------------------------------------|
| `page`    | `integer`        | `1`     | The page number to fetch (1-indexed). Server clamps to `1..=10000`. |
| `limit`   | `integer`        | `20`    | Max items per page (server caps at 100).                           |

Values outside those ranges are clamped rather than rejected, so an
out-of-range page yields an empty list instead of an error. Because the offset
is `(page - 1) * limit`, the page ceiling keeps deep-OFFSET scans bounded and
prevents integer overflow in the offset computation.

Response wrapper:

```json
{
  "data": [ ... ],
  "cursor": "2"   // The next page number as a string, or null when no more pages
}
```

Small, bounded catalogs (`/tags`, `/ingredients`) are not paginated but still use the
`{"data": [...]}` envelope (`CatalogResponse<T>`), without a `cursor` field.

### Error Envelope

All error responses use:

```json
{
  "error": {
    "code": "NOT_FOUND",
    "message": "Recipe with id 42 does not exist."
  }
}
```

Standard status codes used: `400` (validation), `401` (unauthenticated), `403` (forbidden), `404` (not found), `409` (conflict), `422` (unprocessable), `500` (internal).

### Timestamps

All timestamps are ISO 8601 UTC strings using the `+00:00` offset form
(`"2026-01-15T14:30:00+00:00"`).

---

## Authentication & Account

### `POST /api/v1/auth/register`

Create a new user account.

**Request:**
```json
{
  "name": "Kolja",
  "email": "kolja@example.com",
  "password": "password123"
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
  "password": "password123"
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

### `POST /api/v1/auth/logout`

Revoke a refresh token. Requires authentication (Bearer access token).

**Request:**
```json
{
  "refreshToken": "dGhpcyBpcyBhIHJlZnJlc2g..."
}
```

**Response:** `204 No Content`

---

### `DELETE /api/v1/auth/sessions`

Revoke **all** refresh tokens for the authenticated user. Requires authentication.

**Response:** `204 No Content`

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

### `PUT /api/v1/users/me`

Full replacement of the authenticated user's profile core fields. Unlike `PATCH`,
both `name` and `email` are **required** — omitting either one fails body
deserialization rather than silently keeping the old value.

**Request:**
```json
{
  "name": "Kolja M.",
  "email": "kolja@example.com"
}
```

**Response `200 OK`:** *(returns the full updated user object, same shape as `GET /users/me`)*

**Errors:**
- `422 Unprocessable` — `name` or `email` empty.

---

### `PUT /api/v1/users/me/profile-picture`

Upload or replace the user's profile picture. Body is the raw image binary (`Content-Type: image/jpeg`, `image/png`, or `image/webp`).

The upload is also registered in the `images` table (`owner_id` = the user), so the picture is retrievable through `GET /api/v1/images/:hash` as well.

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

| Parameter | Type      | Required | Description                                                                                  |
|-----------|-----------|----------|----------------------------------------------------------------------------------------------|
| `q`       | `string`  | yes      | Search query (min 1 char). Matches against user `name` (case-insensitive, prefix/substring). |
| `limit`   | `integer` | no       | Max results (default `10`, server cap `50`).                                                 |

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
- `400 Bad Request` — the `q` query parameter is missing entirely. Rejected by the axum `Query` extractor *before* the handler runs, so the body is the extractor's own message (`Failed to deserialize query string: missing field q`), not the usual error envelope.
- `422 Unprocessable` — `q` is present but empty (`?q=`); the handler rejects it with `"q cannot be empty"`.

---

## Bulk User Lookup

### `GET /api/v1/users?ids=2,3,5`

Resolve a set of user ids to compact display data in a single request. The frontend uses this to render `owner`, `editors`, `viewers`, and `rating[].user` names from the id references returned by the recipe endpoints (replaces the client-side `listUsers` mock).

**Query parameters:**

| Parameter | Type     | Required | Description                                                        |
|-----------|----------|----------|--------------------------------------------------------------------|
| `ids`     | `string` | yes      | Comma-separated user ids, e.g. `2,3,5`. Whitespace around ids is ignored. |

Malformed segments are **dropped silently** (`ids=1,abc,3` → `[1, 3]`). A missing or empty `ids` parameter returns an empty list rather than an error.

**Bounds:** at most **100** ids per request, and the raw `ids` parameter may not
exceed **2048 bytes** (`422 Unprocessable` beyond either limit — the byte cap
keeps the split/parse work bounded).

**Response `200 OK`:**
```json
{
  "data": [
    { "id": 2, "name": "Mara", "profilePicture": "c4f1..." },
    { "id": 3, "name": "Jonas", "profilePicture": null }
  ]
}
```

> Same compact shape as `GET /users/search` — `id`, `name`, `profilePicture` only. Email addresses are **never** included. Unknown ids are omitted from the result. Order is not guaranteed.

**Errors:**
- `401 Unauthorized` — missing/invalid access token.
- `422 Unprocessable` — more than 100 ids, or an `ids` parameter longer than 2048 bytes.

---

## Recipes

### `GET /api/v1/recipes`

Fetch a paginated list of all recipes accessible to the authenticated user. This is a fast, highly-cacheable endpoint designed for the initial catalog load. It internally uses the same logic as the search endpoint but accepts only pagination parameters in the query string.

**Query parameters:**
- `page` (integer, default: 1, max: 10_000)
- `limit` (integer, default: 20, max: 100)

**Response `200 OK`:** *(Returns the same paginated `RecipePreview` list structure as `POST /api/v1/recipes/search`)*

---

### `POST /api/v1/recipes/search`

Search and list recipes accessible to the authenticated user (owned + shared as editor/viewer). Allows filtering by tags, categories, ingredients, max work time, access rights, and share states. Also supports dynamic sorting (e.g. by rating, name, total time) and uses offset-based pagination. Returns **preview data** — a subset of fields optimized for the list view. Sections, ingredients, steps, and notes are **not** included. If the request body is omitted or empty (`{}`), it behaves identically to a fetch-all endpoint.

**Query parameters:**
- `page` (integer, default: 1, max: 10_000)
- `limit` (integer, default: 20, max: 100)

**Request:**
```json
{
  "filters": {
    "categories": [1, 2],
    "tags": ["Hauptgericht", "Vegan"],
    "ingredients": [5],
    "maxWorkTime": 30,
    "accessRights": ["owner", "editor", "viewer"],
    "shareStates": ["private", "shared", "collaborative"]
  },
  "sort": {
    "field": "rating", // "name", "workTime" | "work_time", "totalTime" | "total_time", "rating", "accessSince"
    "order": "desc"    // "asc", "desc"
  }
}
```
*(All fields are optional. Filters use strict AND logic where arrays are provided. The ID/string filter arrays (`categories`, `tags`, `ingredients`) accept at most **100** entries; the enum filter arrays (`accessRights`, `shareStates`) accept at most **10** entries (duplicates are deduped and there are only three variants each). Requests exceeding a cap are rejected with `422 Unprocessable` (`"categories accepts at most 100 entries"`, `"accessRights accepts at most 10 entries"`, etc.). Sort fields are case-sensitive; the snake_case spellings `work_time` and `total_time` are accepted as aliases of `workTime` and `totalTime`.)*

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
      "rating": [],
      "time": "45 min",
      "workMinutes": 20,
      "overallMinutes": 45,
      "sizeNumber": 4,
      "sizeText": "{Portionen}",
      "mainImage": { "id": 1, "hash": "a3f8c2d1e5...", "name": null },
      "createdAt": "2026-01-10T12:00:00+00:00",
      "updatedAt": "2026-03-15T09:30:00+00:00",
      "myAccessSince": "2026-02-01T08:15:00+00:00"
    }
  ],
  "cursor": "2"
}
```

> **Scope:** The backend **only returns recipes where the user is `owner`, `editor`, or `viewer`**. The frontend does not need to filter by access itself. The `cursor` in the response represents the next page number as a string, or `null` if there are no more pages.

> **`myAccessSince`:** when the *requesting* user gained access to the recipe. For the `owner` it equals `createdAt`; for an `editor`/`viewer` it is the join-table `added_at` timestamp (editor wins if a user appears in both lists). Use it for "recently shared with you" ordering; `sort.field = "accessSince"` orders by the same value.

**Errors:**
- `400 Bad Request` — malformed query string (`page` / `limit` not integers), or a request body that is not valid JSON syntax. Both are rejected by the axum extractors before the handler runs.
- `415 Unsupported Media Type` — a body was sent without `Content-Type: application/json`.
- `401 Unauthorized` — missing/invalid access token.
- `422 Unprocessable` — a filter array exceeds its cap (see above), a body field has the wrong type, or `sort.field` is not a known field. The sort error message lists the accepted spellings and repeats the rejected value, e.g. `sort.field must be one of name, workTime, work_time, totalTime, total_time, rating, accessSince (got "worktime")`. Unknown sort fields are rejected rather than silently falling back to ordering by id.

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
  "rating": [],
  "time": "45 min",
  "workMinutes": 20,
  "overallMinutes": 45,
  "sizeNumber": 4,
  "sizeText": "{Portionen}",
  "notes": ["Am besten mit frischem Basilikum servieren."],
  "mainImage": { "id": 1, "hash": "a3f8c2d1e5...", "name": null },
  "images": [
    { "id": 2, "hash": "b2e4f6a8c1...", "name": null },
    { "id": 3, "hash": "c3d5e7f9b2...", "name": null }
  ],
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
  "createdAt": "2026-01-10T12:00:00+00:00",
  "updatedAt": "2026-03-15T09:30:00+00:00",
  "myAccessSince": "2026-01-10T12:00:00+00:00"
}
```

**Errors:**
- `404 Not Found` — recipe does not exist.
- `403 Forbidden` — user is not owner/editor/viewer.

> `myAccessSince` follows the same semantics as in the previews: owner → `createdAt`, editor/viewer → the join-table `added_at`.

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

> The same id → object expansion applies to images: requests send `mainImage` / `images`
> as image ids (or `null` / `[]`), while responses return full `ImageRef` objects
> `{ id, hash, name }` (`mainImage` is `null` when unset).

**Response `200 OK`:** *(full recipe object, same shape as `GET /recipes/:id`)*

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

**Response `200 OK`:** *(full recipe object of the new copy)*

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

**Query parameters:**

| Parameter | Type     | Required | Description                                                        |
|-----------|----------|----------|--------------------------------------------------------------------|
| `page`    | `int`    | no       | 1-based page number (default: `1`).                                |
| `limit`   | `int`    | no       | Page size (default: `20`, max: `100`).                             |

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
  ],
  "cursor": "2"
}
```

> `cursor` is the next `page` number when more data is available, otherwise `null`. Pass it back as `page` to fetch the next page.

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

**Response `200 OK`:** *(full category object; `recipes` starts as `[]`, `order` is auto-assigned)*

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

**Bound:** at most **500** recipe IDs per request (`422 Unprocessable` beyond that).

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
- `422 Unprocessable` — recipe IDs that the user doesn't have access to, or more than 500 recipe IDs in the request.

---

### `PUT /api/v1/categories/order`

Reorder all categories at once. Expects the full list of category IDs in the desired order.

**Bound:** at most **100** category IDs per request (`422 Unprocessable` beyond that).

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

Upload an image. **Requires authentication** (`Authorization: Bearer <token>`). The uploader is recorded as `owner_id`. Body is the raw image binary (`Content-Type: image/jpeg`, `image/png`, or `image/webp`) — not multipart. The format is detected from magic bytes, not the request header. Max body size: 8 MiB.

**Response `201 Created`** for a new image, **`200 OK`** when re-uploading identical
bytes (the content hash already exists, so the existing row is returned — the upload
is idempotent):
```json
{
  "id": 7,
  "hash": "a3f8c2d1e5...",
  "name": null
}
```

**Errors:**
- `401 Unauthorized` — missing or invalid access token.
- `422 Unprocessable Entity` — empty body, or unsupported format (only JPEG, PNG, WEBP).

---

### `GET /api/v1/images/:hash`

Retrieve an image by its content hash. **Public** — no authentication required, so `<img src>` tags work directly. Returns the raw image binary with the appropriate `Content-Type` header. Supports `If-None-Match` / `ETag` for caching (the hash *is* the ETag).

**Response `200 OK`:** *(binary image data)*

**Headers:**
```
Content-Type: image/jpeg
ETag: "a3f8c2d1e5..."
Cache-Control: public, max-age=31536000, immutable
```

> Because images are content-addressed (hash = filename), they are immutable — aggressive caching is safe.

**Errors:**
- `304 Not Modified` — `If-None-Match` matched the current ETag.
- `404 Not Found` — no image with this hash (or malformed hash).

---

### `DELETE /api/v1/images/:hash`

Delete an image by its content hash from both the database and the server's disk.

**Requires authentication** and the caller must be the uploader (owner). Non-owners receive `404` — the existence of someone else's image is never revealed (`404` rather than `403` is intentional).

If the image is still referenced (a user's profile picture, a recipe main image, or a recipe gallery image), the image is kept: the row (including `owner_id`), the file, and the references stay untouched, so the owner can retry the delete once the last reference is gone.

**Response `204 No Content`**

**Errors:**
- `401 Unauthorized` — missing or invalid access token.
- `404 Not Found` — no image with this hash **or** the caller is not the owner.

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

### `PUT /api/v1/tags/:id/icon`

Upload a new SVG icon for a tag. The body must be a valid SVG file.

**Response `204 No Content`**

**Errors:**
- `422 Unprocessable Entity` — empty body or invalid SVG format.
- `404 Not Found` — tag does not exist.

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
      "timestamp": "2026-06-28T10:00:00+00:00",
      "data": { "name": "Spaghetti Bolognese (updated)" }
    },
    {
      "type": "recipe.create",
      "tempId": "tmp-abc-123",
      "timestamp": "2026-06-28T10:01:00+00:00",
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

| Type               | Direction    | Description                                                                         |
|--------------------|--------------|-------------------------------------------------------------------------------------|
| `me`               | request      | Get current user profile                                                            |
| `recipes.list`     | request      | List accessible recipes (preview data)                                              |
| `recipes.get`      | request      | Get full recipe by ID                                                               |
| `categories.list`  | request      | List user's categories                                                              |
| `tags.list`        | request      | List all tags                                                                       |
| `ingredients.list` | request      | List ingredient catalog                                                             |
| `users.list`       | request      | List all users (for catalog/invite dropdown, deprecated in favor of `users.search`) |
| `users.search`     | request      | Search users by name for invite dropdowns                                           |
| `recipe.updated`   | server event | A recipe the client is subscribed to was changed                                    |
| `recipe.deleted`   | server event | A recipe was deleted                                                                |
| `sync.submit`      | request      | Submit offline change backlog                                                       |

---

## Configuration & Deployment

### Environment Variables

| Variable                | Default                     | Description                                                                 |
|-------------------------|-----------------------------|-----------------------------------------------------------------------------|
| `JWT_SECRET`            | *(auto-generated)*           | Secret used to sign access tokens. Generated on first boot if not set.      |
| `PASSWORD_PEPPER`       | *(auto-generated)*           | Argon2id pepper. Generated on first boot if not set.                        |
| `JWT_EXPIRY_MINUTES`    | `15`                        | Access-token lifetime in minutes. Refresh tokens rotate on use.             |
| `REFRESH_TOKEN_EXPIRY_DAYS` | `30`                    | Refresh-token lifetime in days.                                            |
| `RATE_LIMIT_MODE`       | `direct`                    | `direct` \| `proxy` \| `disabled` — see below.                              |
| `RATE_LIMIT_PER_SECOND` | `12`                        | Replenish interval in seconds per rate-limit slot (u32).                     |
| `RATE_LIMIT_BURST_SIZE` | `5`                         | Maximum burst of requests before throttling (u32).                           |
| `TRUSTED_PROXIES`       | *(empty)*                   | Comma-separated proxy IPs/CIDRs trusted in `proxy` mode.                    |
| `CORS_ALLOWED_ORIGINS`  | `http://localhost:5173`     | Comma-separated allowed browser origins.                                    |
| `IMAGE_STORAGE_PATH`    | *(app default)*             | Root directory for content-addressed image files.                           |
| `DATABASE_URL`          | *(required)*                | PostgreSQL connection string.                                               |

### Rate limiting topology (`RATE_LIMIT_MODE`)

The auth endpoints (`register`, `login`, `refresh`) are rate limited per client IP
(default 5/min sustained, burst 5; tunable via `RATE_LIMIT_PER_SECOND` and
`RATE_LIMIT_BURST_SIZE`). How the client IP is resolved depends on deployment:

- **`direct`** — the API is exposed directly. Only the TCP peer IP is used;
  `X-Forwarded-For` / `X-Real-IP` are ignored so a client cannot spoof a fresh
  bucket per request.
- **`proxy`** — the API sits behind a reverse proxy (nginx, Caddy, a load
  balancer). The **rightmost** `X-Forwarded-For` entry is used (the one appended
  by our own proxy), then `X-Real-IP`, then the peer IP. The rightmost entry is
  chosen because anything to its left is client-controlled and spoofable.
  `TRUSTED_PROXIES` lists the proxies whose appended entry may be trusted.
- **`disabled`** — the proxy already rate limits; the API does none. Use this
  only when rate limiting is genuinely handled upstream.

An unknown value fails fast at startup rather than silently choosing a topology.

### HTTPS / production guidance

**Production must serve the API over HTTPS.** JWTs (access + refresh tokens) are
bearer credentials; over plain HTTP anyone on the path can replay them.

Recommended setup: TLS terminates at nginx, which proxies to the app on
`127.0.0.1`/localhost:

```nginx
server {
    listen 443 ssl http2;
    server_name api.example.com;

    ssl_certificate     /etc/letsencrypt/live/api.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/api.example.com/privkey.pem;

    client_max_body_size 10M;   # image uploads are capped at 8 MiB in-app

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host              $host;
        proxy_set_header X-Real-IP         $remote_addr;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}

server {
    listen 80;
    server_name api.example.com;
    return 301 https://$host$request_uri;
}
```

With that topology set:

```bash
RATE_LIMIT_MODE=proxy
TRUSTED_PROXIES=127.0.0.1,::1
CORS_ALLOWED_ORIGINS=https://app.example.com
```

- `proxy_add_x_forwarded_for` **appends** the real client IP to any client-supplied
  `X-Forwarded-For`; the app trusts only the rightmost entry.
- If the API is exposed directly (no proxy), use `RATE_LIMIT_MODE=direct` and do
  not send forwarded headers.
- `CORS_ALLOWED_ORIGINS` should list only the real frontend origins — never `*` —
  and must be HTTPS origins in production.
- `TRUSTED_PROXIES` should be the narrowest set of proxy addresses possible.

---

## Endpoint Summary

| Method   | Path                                  | Auth | Description                                      |
|----------|---------------------------------------|------|--------------------------------------------------|
| `POST`   | `/api/v1/auth/register`               | no   | Create account                                   |
| `POST`   | `/api/v1/auth/login`                  | no   | Login, get tokens                                |
| `POST`   | `/api/v1/auth/refresh`               | no   | Refresh access token                             |
| `POST`   | `/api/v1/auth/logout`                | yes  | Logout (revoke refresh token)                    |
| `DELETE` | `/api/v1/auth/sessions`              | yes  | Logout all sessions                              |
| `GET`    | `/api/v1/users/me`                    | yes  | Current user profile                             |
| `PATCH`  | `/api/v1/users/me`                    | yes  | Update own profile                               |
| `PUT`    | `/api/v1/users/me`                    | yes  | Replace own profile (name + email required)      |
| `PUT`    | `/api/v1/users/me/profile-picture`    | yes  | Upload/replace profile picture                   |
| `DELETE` | `/api/v1/users/me/profile-picture`    | yes  | Remove profile picture                           |
| `GET`    | `/api/v1/users/search?q=...`          | yes  | Search users by name (invite dropdown)           |
| `GET`    | `/api/v1/users?ids=2,3,5`             | yes  | Bulk user lookup (id → name/picture)             |
| `GET`    | `/api/v1/recipes`                     | yes  | List recipes (preview, paginated)                |
| `POST`   | `/api/v1/recipes/search`              | yes  | Search/filter/sort recipes (preview, paginated)  |
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
| `POST`   | `/api/v1/images`                      | yes  | Upload image (records owner)                     |
| `GET`    | `/api/v1/images/:hash`                | no   | Get image by hash (public, cacheable)            |
| `DELETE` | `/api/v1/images/:hash`                | yes  | Delete image (owner only; non-owner 404)         |
| `POST`   | `/api/v1/sync`                        | yes  | Submit offline change backlog                    |
