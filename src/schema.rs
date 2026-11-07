// @generated automatically by diesel-cli

diesel::table! {
    users (id) {
        id -> BigInt,
        name -> Text,
        email -> Text, // citext
        password_hash -> Text,
        profile_picture -> Nullable<Text>,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    tags (id) {
        id -> Text,
        svg -> Nullable<Text>,
    }
}

diesel::table! {
    ingredients (id) {
        id -> BigInt,
        name -> Text,
    }
}

diesel::table! {
    images (id) {
        id -> BigInt,
        hash -> Text,
        name -> Nullable<Text>,
        owner_id -> Nullable<BigInt>,
    }
}

diesel::table! {
    recipes (id) {
        id -> BigInt,
        owner_id -> BigInt,
        name -> Text,
        source -> Nullable<Text>,
        time_display -> Nullable<Text>,
        work_minutes -> Nullable<Integer>,
        overall_minutes -> Nullable<Integer>,
        size_number -> Nullable<Integer>,
        size_text -> Nullable<Text>,
        notes -> Array<Nullable<Text>>,
        main_image_id -> Nullable<BigInt>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    sections (id) {
        id -> BigInt,
        recipe_id -> BigInt,
        name -> Nullable<Text>,
        position -> Integer,
    }
}

diesel::table! {
    steps (id) {
        id -> BigInt,
        section_id -> BigInt,
        text -> Text,
        position -> Integer,
    }
}

diesel::table! {
    recipe_ingredients (id) {
        id -> BigInt,
        section_id -> BigInt,
        ingredient_id -> Nullable<BigInt>,
        text -> Nullable<Text>,
        amount -> Nullable<Text>,
        amount_prefix -> Nullable<Text>,
        unit -> Nullable<Text>,
        position -> Integer,
    }
}

diesel::table! {
    recipe_editors (recipe_id, user_id) {
        recipe_id -> BigInt,
        user_id -> BigInt,
    }
}

diesel::table! {
    recipe_viewers (recipe_id, user_id) {
        recipe_id -> BigInt,
        user_id -> BigInt,
    }
}

diesel::table! {
    recipe_tags (recipe_id, tag_id) {
        recipe_id -> BigInt,
        tag_id -> Text,
        position -> Integer,
    }
}

diesel::table! {
    recipe_images (recipe_id, image_id) {
        recipe_id -> BigInt,
        image_id -> BigInt,
        position -> Integer,
    }
}

diesel::table! {
    user_ratings (recipe_id, user_id) {
        recipe_id -> BigInt,
        user_id -> BigInt,
        rating -> SmallInt,
    }
}

diesel::table! {
    user_categories (id) {
        id -> BigInt,
        user_id -> BigInt,
        name -> Text,
        sort_order -> Nullable<Integer>,
        color -> Text,
        color_light -> Nullable<Text>,
        color_dark -> Nullable<Text>,
    }
}

diesel::table! {
    user_category_recipes (category_id, recipe_id) {
        category_id -> BigInt,
        recipe_id -> BigInt,
        position -> Integer,
    }
}

diesel::table! {
    refresh_tokens (id) {
        id -> BigInt,
        user_id -> BigInt,
        token_hash -> Text,
        expires_at -> Timestamptz,
        created_at -> Timestamptz,
    }
}

diesel::table! {
    ws_tickets (id) {
        id -> BigInt,
        user_id -> BigInt,
        ticket -> Uuid,
        expires_at -> Timestamptz,
        used -> Bool,
    }
}

diesel::table! {
    groups (id) {
        id -> BigInt,
        name -> Text,
        owner_id -> BigInt,
    }
}

diesel::table! {
    group_members (group_id, user_id) {
        group_id -> BigInt,
        user_id -> BigInt,
    }
}

// Foreign-key relations used by DSL joins in the codebase.
diesel::joinable!(recipe_ingredients -> sections (section_id));
diesel::joinable!(recipe_ingredients -> ingredients (ingredient_id));
diesel::joinable!(recipe_images -> images (image_id));

diesel::allow_tables_to_appear_in_same_query!(
    users,
    tags,
    ingredients,
    images,
    recipes,
    sections,
    steps,
    recipe_ingredients,
    recipe_editors,
    recipe_viewers,
    recipe_tags,
    recipe_images,
    user_ratings,
    user_categories,
    user_category_recipes,
    refresh_tokens,
    ws_tickets,
    groups,
    group_members,
);
