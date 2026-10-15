//! Font menus share saved favorites. Menu changes are applied once through prefs.set after
//! drawing, so every picker (including Character, styles and Glyphs) uses the same list.

use egui::{Context, Id};

fn id() -> Id {
    Id::new("photocraft-font-favorites")
}

pub(crate) fn sync(ctx: &Context, favorites: &[String]) {
    ctx.data_mut(|d| d.insert_temp(id(), favorites.to_vec()));
}

pub(crate) fn get(ctx: &Context) -> Vec<String> {
    ctx.data(|d| d.get_temp(id())).unwrap_or_default()
}

pub(crate) fn toggle(ctx: &Context, family: &str) {
    let mut favorites = get(ctx);
    if favorites.iter().any(|f| f == family) {
        favorites.retain(|f| f != family);
    } else if favorites.len() < 256 && !family.trim().is_empty() && family.len() <= 256 {
        favorites.push(family.to_string());
    } else {
        return;
    }
    ctx.data_mut(|d| {
        d.insert_temp(id(), favorites.clone());
        d.insert_temp(id().with("pending"), favorites);
    });
}

pub(crate) fn persist(app: &mut crate::PhotocraftApp, ctx: &Context) {
    if let Some(favorites) = ctx.data_mut(|d| d.remove_temp::<Vec<String>>(id().with("pending")))
        && let Err(error) = app.run("prefs.set", serde_json::json!({"values": {"type.favoriteFonts": favorites}}))
    {
        app.ui.status = error;
    }
}
