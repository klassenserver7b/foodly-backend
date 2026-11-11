# 03 - Data Models & Storage

## Storage Policies

1. **Image Storage**: Images are stored locally on the server's disk. The database stores only the file hash (which acts
   as the filename). Files are stored in a nested folder structure (e.g., `images/ab/cd/abcdef...`) to avoid directory
   overload. Since images are content-addressed, they are immutable — the same hash always yields the same file.

2. **Pagination & Previews**: List endpoints (like `GET /api/v1/recipes`) use **offset-based** pagination via the
   `page` / `limit` query parameters (`page` defaults to 1, `limit` to 20, capped at 100; the SQL offset is
   `(page - 1) * limit`). Responses are wrapped as `{ "data": [...], "cursor": "2" | null }`, where `cursor` carries the
   **next page number** as a string (the field name the frontend already consumes) and is `null` on the last page —
   despite the name, nothing is cursor-encoded. The recipes list endpoint returns only preview data (no sections,
   ingredients, or steps), requiring a fetch to `GET /api/v1/recipes/:id` for full details. Small bounded catalogs
   (tags, ingredients) are returned as flat arrays without pagination.

3. **Timestamps**: All entities that support creation/update tracking use ISO 8601 UTC strings. `updatedAt` on recipes
   reflects content changes only — not permission/sharing changes.

---

## Domain Models

### User

| Field            | Type           | Notes                                                                    |
|------------------|----------------|--------------------------------------------------------------------------|
| `id`             | `UserId` (int) | Server-assigned, auto-increment.                                         |
| `name`           | `string`       | Display name. Unique is **not** enforced (users are identified by `id`). |
| `email`          | `string`       | Unique. Used for login.                                                  |
| `profilePicture` | `Hash \| null` | Content hash of the profile image; `null` if none set.                   |
| `passwordHash`   | `string`       | *(DB-only, never exposed via API.)*                                      |
| `createdAt`      | `timestamp`    | Account creation date.                                                   |

### Recipe

| Field            | Type              | Notes                                                                           |
|------------------|-------------------|---------------------------------------------------------------------------------|
| `id`             | `RecipeId` (int)  | Server-assigned.                                                                |
| `owner`          | `UserId`          | Single owner. Drives "my recipes" in the frontend.                              |
| `editors`        | `UserId[]`        | Users who can edit. Any viewer/editor/owner can create a personal copy.         |
| `viewers`        | `UserId[]`        | Users who can view.                                                             |
| `name`           | `string`          | Recipe title. May contain `{tagId}` tokens for inline tag rendering.            |
| `tags`           | `TagId[]`         | Array of tag IDs (= tag names).                                                 |
| `source`         | `string \| null`  | URL or freetext source attribution.                                             |
| `rating`         | `UserRating[]`    | *(Embedded on read; stored as separate DB rows.)*                               |
| `time`           | `string \| null`  | Human-readable time string (may contain `{tagId}` tokens).                      |
| `workMinutes`    | `int \| null`     | Active working time in minutes (for filtering).                                 |
| `overallMinutes` | `int \| null`     | Total time including waiting (for filtering).                                   |
| `sizeNumber`     | `int \| null`     | Editable portion count (e.g. `3` for "3 Portionen"). `null` = fixed descriptor. |
| `sizeText`       | `string \| null`  | TagText-formatted label (e.g. `"{Portionen}"`, `"28 cm {Springform}"`).         |
| `notes`          | `string[]`        | Free-text notes. May contain `{tagId}` tokens.                                  |
| `mainImage`      | `ImageRef \| null` | Hero image for list thumbnail / detail header. `null` = none.                   |
| `images`         | `ImageRef[]`      | Additional gallery images (does not include `mainImage`).                       |
| `sections`       | `Section[]`       | *(Embedded in full responses; omitted in list previews.)*                       |
| `createdAt`      | `timestamp`       | Recipe creation date.                                                           |
| `updatedAt`      | `timestamp`       | Last **content** update (not sharing/permission changes).                       |

> **Images on read vs. write.** `Recipe` and `RecipePreview` responses embed expanded image references —
> `mainImage: { id, hash, name } | null` and `images: [{ id, hash, name }]` — so the frontend can build the serve URL
> (`GET /images/{hash}`) without a second request. Write bodies (`CreateRecipe`) send bare ids instead:
> `mainImage: ImageId | null` and `images: ImageId[]`. The server expands ids to `ImageRef` objects on the way out.

### Section

| Field         | Type                 | Notes                                      |
|---------------|----------------------|--------------------------------------------|
| `id`          | `SectionId` (int)    | Server-assigned. Unique within the recipe. |
| `name`        | `string \| null`     | Optional section heading (e.g. "Teig").    |
| `ingredients` | `RecipeIngredient[]` | Ordered list.                              |
| `steps`       | `string[]`           | Ordered instruction steps (plain text).    |

### Step (DB-only)

| Field  | Type           | Notes                                       |
|--------|----------------|---------------------------------------------|
| `id`   | `StepId` (int) | Server-assigned. Unique within the section. |
| `text` | `string`       | The instruction text.                       |

> Steps are stored as their own DB rows (not a flat `string[]`) so that per-step
> metadata — like `duration` for recipe-linked timers — can be added later with a
> simple column addition instead of a data migration.
>
> On the wire, however, `Section.steps` is a plain `string[]`: the API serializes
> only the step text, not the row shape. `Step.id` never leaves the server. When
> `Step` gains client-relevant fields (e.g. `duration?: number`), the wire type can
> grow into an object incrementally.

### RecipeIngredient

| Field          | Type                       | Notes                                                                                                             |
|----------------|----------------------------|-------------------------------------------------------------------------------------------------------------------|
| `id`           | `RecipeIngredientId` (int) | Server-assigned. Unique within the recipe.                                                                        |
| `ingredient`   | `IngredientRef \| null`    | Expanded reference `{ id, name }` on read; just `IngredientId` (or `null`) on write. `null` = pure freetext line. |
| `text`         | `string \| null`           | If `ingredient` is set: suffix rendered after the name. If `null`: the freetext ingredient line itself.           |
| `amount`       | `string \| null`           | Quantity as string (e.g. `"500"`, `"2-3"`). String to preserve formatting.                                        |
| `amountPrefix` | `string \| null`           | Prefix before amount (e.g. `"ca."`).                                                                              |
| `unit`         | `string \| null`           | Unit string (e.g. `"g"`, `"EL"`, `"Stück"`).                                                                      |

### Ingredient (Catalog)

| Field  | Type                 | Notes                            |
|--------|----------------------|----------------------------------|
| `id`   | `IngredientId` (int) | Server-assigned.                 |
| `name` | `string`             | Display name (e.g. "Spaghetti"). |

> The ingredient catalog is a **fixed global list**, curated server-side. Users cannot add to it.  
> Future: additional metadata (icon, default unit, nutritional values) may be added here.

### IngredientRef (Response-only)

The expanded inline reference in each `RecipeIngredient` response. Carries the `id` (for shopping list / filtering) and
the `name` (so the frontend can render without fetching the catalog).

```
{ id: IngredientId, name: string }
```

### Tag

| Field | Type             | Notes                                                    |
|-------|------------------|----------------------------------------------------------|
| `id`  | `TagId` (string) | The tag name **is** the ID (e.g. `"Hauptgericht"`).      |
| `svg` | `Hash \| null`   | Hash/filename of an optional SVG icon. `null` = no icon. |

> **Important:** Because `tagId = name`, renaming a tag would break all references. If tags ever become editable, a
> separate stable `id` and `label` will be needed.

### UserRating (DB-only)

| Field    | Type       | Notes         |
|----------|------------|---------------|
| `recipe` | `RecipeId` | FK to recipe. |
| `user`   | `UserId`   | FK to user.   |
| `rating` | `int`      | 1–5.          |

> Never exposed as a standalone wire type. The frontend only sees ratings as `recipe.rating[]`.

### Image

| Field  | Type             | Notes                                           |
|--------|------------------|-------------------------------------------------|
| `id`   | `ImageId` (int)  | Server-assigned.                                |
| `hash` | `Hash` (string)  | Content hash; doubles as filename on disk.      |
| `name` | `string \| null` | Optional display name (e.g. "Fertiger Teller"). |

> The DB row also carries `owner_id` (uploader, used for delete authorization), which is **not** exposed on the wire.

### ImageRef (Response-only)

The image shape embedded in `Recipe` / `RecipePreview` (`mainImage`, `images`). It carries only what the frontend
needs to render — the `hash` for the serve URL and the display `name` — and omits server-only columns such as
`owner_id`.

```
{ id: ImageId, hash: Hash, name: string | null }
```

### UserCategory

| Field        | Type                   | Notes                                        |
|--------------|------------------------|----------------------------------------------|
| `id`         | `UserCategoryId` (int) | Server-assigned.                             |
| `user`       | `UserId`               | Owner (each user has their own categories).  |
| `name`       | `string`               | Display name (e.g. "Favoriten").             |
| `recipes`    | `RecipeId[]`           | Ordered list of recipe IDs in this category. |
| `order`      | `int \| null`          | Sort position among the user's categories.   |
| `color`      | `string`               | Primary color (hex, e.g. `"#e11d48"`).       |
| `colorLight` | `string \| null`       | Optional light-mode override color.          |
| `colorDark`  | `string \| null`       | Optional dark-mode override color.           |

### Group

| Field     | Type            | Notes                                                 |
|-----------|-----------------|-------------------------------------------------------|
| `id`      | `GroupId` (int) | Server-assigned.                                      |
| `name`    | `string`        | Display name (e.g. "Familie").                        |
| `owner`   | `UserId`        | Creator of the group. Only the owner can modify it.   |
| `members` | `UserId[]`      | List of member user IDs (does not include the owner). |

> **Groups ↔ Recipes:** Groups are **not** linked to recipes in the data model. When sharing a recipe via a group, the
> frontend expands the group's members and adds them individually to the recipe's `editors`/`viewers`. This means adding
> someone to a group later does **not** retroactively grant them access to previously-shared recipes. This trade-off is
> documented and may be revisited.

---

## Permissions Matrix

| Action                 | Owner | Editor | Viewer | Other |
|------------------------|-------|--------|--------|-------|
| View recipe            | ✅     | ✅      | ✅      | ❌     |
| Edit recipe content    | ✅     | ✅      | ❌      | ❌     |
| Delete recipe          | ✅     | ❌      | ❌      | ❌     |
| Manage editors/viewers | ✅     | ❌      | ❌      | ❌     |
| Rate recipe            | ✅     | ✅      | ✅      | ❌     |
| Copy recipe            | ✅     | ✅      | ✅      | ❌     |

---

## Collaboration States (derived, not stored)

The frontend derives a recipe's collaboration state from its permission arrays:

| State           | Condition                                      |
|-----------------|------------------------------------------------|
| `private`       | `editors.length === 0 && viewers.length === 0` |
| `shared`        | `editors.length === 0 && viewers.length > 0`   |
| `collaborative` | `editors.length > 0`                           |

This drives UI behavior (e.g. rating display: own rating vs. pooled average).

---

## Open Questions (to resolve with frontend)

- **Timestamps**: `createdAt` and `updatedAt` are needed for dashboard features (Phase 5: "recently created", "recently
  added to"). `updatedAt` should only reflect content changes, not permission changes.
- **`UserCategory` icon**: Categories may get an optional `icon` field in the future.
- **Notification / activity feed**: Needed to inform users when they're added to a recipe/group. DB schema TBD.

