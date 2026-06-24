type hash = string;

type userCategoryId = number;
type userCategory = {
    id: userCategoryId;
    user: userId;
    name: string;
    recipes: recipeId[];
    order: number | null;
    color: string;
    colorLight: string | null;
    colorDark: string | null;
};

type recipeId = number;
type recipe = {
    id: recipeId;

    owner: userId; // my recipes area im frontend (mit personal filter für alle rezepte die nicht geshared sind)
    viewers: userId[]; // jeder viewer (und editor und owner) kann eine kopie des rezeptes erstellen wo er dann owner ist (viewers und editors sind dort erst leer)
    editors: userId[];

    name: string;
    tags: tagId[];
    source: string | null;

    time: string | null;
    workMinutes: number | null;
    overallMinutes: number | null;
    amount: string | null; // 3 {Portionen}            28 cm {Springform}
    basePortionMultiplier: number | null; // 3                        1
    notes: string[];
    images: imageId[];

    sections: sectionId[];

    // So nicht in der DB gestored, aber von der API returned:
    rating: { user: userId; rating: number }[];
};

type sectionId = number;
type section = {
    id: sectionId;
    name: string | null;
    ingredients: recipeIngredientId[];
    steps: string[];
};

type recipeIngredientId = number;
type recipeIngredient = {
    id: recipeIngredientId;
    ingredient: ingredientId | null;
    text: string | null; // entweder hardcoded text wenn keine ingredientId angegeben, oder suffix der hinter ingredient gerendert wird
    amount: string | null;
    amountPrefix: string | null;
    unit: string | null;
};

type ingredientId = number;
type ingredient = {
    id: ingredientId;
    name: string;
};

type tagId = string;
type tag = {
    id: tagId; // gleichzeitig der name
    svg: hash | null;
};

type userRating = {
    recipe: recipeId;
    user: userId;
    rating: number;
};

type imageId = number;
type image = {
    id: imageId;
    hash: hash;
    name: string | null;
};

type userId = number;
type user = {
    id: userId;
    name: string;
    profilePicture: hash | null;
    // ...
};

type groupId = number;
type group = {
    id: groupId;
    name: string;
    members: userId[];
};
