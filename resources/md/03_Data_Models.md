# 03 - Data Models & Storage

## Storage Policies

1. **Image Storage**: Images will be stored locally on the server's disk. The database will only store the file hash (which acts as the filename). Files will be stored in a smart folder structure (e.g., `images/ab/cd/abcdef...`) to avoid directory overload.
2. **Pagination & Previews**: List endpoints (like `/api/v1/recipes`) will use cursor-based pagination. To minimize traffic, the recipes list endpoint will only return a subset of core data (preview), requiring a fetch to `/api/v1/recipes/:id` for full details.

## Domain Models

### Recipes
- Recipes have a single `owner`. 
- View and edit access is governed by the `editors` and `viewers` lists on the Recipe itself. The backend verifies permissions against these arrays.

### Groups
- A `Group` simply represents a list of user IDs. 
- When a group is added to a recipe, it functions as a bulk assignment, injecting those users directly into the `editors` or `viewers` arrays of the recipe.

### Ingredients & RecipeIngredients
- **Ingredient Catalog**: The `Ingredient` catalog is a fixed global list. Users cannot add to it.
- **RecipeIngredient**: Users can create `RecipeIngredient` entries per recipe. These can contain either a reference to an `Ingredient`, arbitrary freetext, or both. These are scoped strictly to their respective recipe and are not shared globally.
