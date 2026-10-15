//! What Craft AI does inside PhotoCraft. Craft AI is one assistant across the ArtCraft family
//! — the model, the runtime and the shared folder belong to the family — but its *work* is the
//! app's own: the system prompt tells the model which app it is inside and what that app does,
//! and every ready-made task is framed for that app's world, so the answer that comes back is
//! about layers and masks and blend modes, not generic advice.
//!
//! Each app of the family carries its own `tasks.rs` — VectorCraft's asks about paths and
//! Pathfinder steps, WordCraft's about documents and styles, and so on — same model, same
//! shared folder, different craft. The labels and hints are catalog keys: the window shows
//! them translated.

/// The app Craft AI is embedded in, as the shared folder's records name it.
pub const APP: &str = "PhotoCraft";

/// One ready-made task of the Assistant tab.
pub struct Task {
    /// The label the picker shows (a catalog key).
    pub label: &'static str,
    /// The input field's hint (a catalog key).
    pub hint: &'static str,
}

/// The tasks, in the picker's order.
pub const TASKS: &[Task] = &[
    Task { label: "Ask anything", hint: "Ask in any language" },
    Task { label: "Retouch plan", hint: "The flaws to fix — spots, wrinkles, objects, noise" },
    Task { label: "Layers and masks strategy", hint: "What the image is made of, and what should change" },
    Task { label: "Menu path finder", hint: "What you want to do, in your own words" },
    Task { label: "Export recipe", hint: "Where the file goes and what matters (size, quality, format)" },
    Task { label: "Alt text for the image", hint: "The image and who the text is for" },
];

/// Tell the model where it is and how to answer: briefly, practically, in the language the user
/// wrote in, about this app's own world — the words of its interface, the files it opens and
/// writes. The last line is the honesty clause: a local model can be wrong, and it is told to
/// say so rather than invent.
pub fn system_prompt() -> String {
    "You are Craft AI, the assistant built into PhotoCraft, a desktop image editor. \
     PhotoCraft's world, in its own words: layers, layer groups and blend modes, layer masks \
     and clipping masks; selections — marquee, lasso, magic wand, Select and Mask; the \
     retouching tools — Spot Healing, Healing Brush, Patch, Clone Stamp, Content-Aware Move; \
     adjustments and adjustment layers — Levels, Curves, Hue/Saturation, the Camera Raw \
     filter; smart objects and smart filters; filters and the Filter Gallery; transforms, \
     crop and perspective crop; the panels — Layers, Channels, Paths, History, Properties, \
     Adjustments, Navigator, Histogram; brushes and gradients; opening camera files, and \
     exporting — Export As, Save for Web. Answer in the language the user wrote in, Arabic \
     included. Be brief and practical: name the tool or the menu path, give the steps as a \
     short list, say what to look at to check it worked, and when a question is not about \
     PhotoCraft or image editing, answer it anyway, briefly. Never invent menu entries, tool \
     names or panel names: if you are not sure PhotoCraft has a feature, say so plainly."
        .to_string()
}

/// Frame the user's input for the task they picked.
pub fn user_prompt(task: usize, input: &str) -> String {
    let input = input.trim();
    match TASKS[task.min(TASKS.len() - 1)].label {
        "Retouch plan" => format!(
            "Plan the retouch of this image in PhotoCraft — which tools in what order, on what layer, and what to leave alone: {input}"
        ),
        "Layers and masks strategy" => format!(
            "Design the layer and mask structure for this edit in PhotoCraft — what goes on its own layer, what a mask covers, and why: {input}"
        ),
        "Menu path finder" => format!(
            "Find the PhotoCraft menu paths for this — the menu item, the tool, and any options along the way: {input}"
        ),
        "Export recipe" => format!(
            "Give an export recipe for this image in PhotoCraft — the format, the settings and the trade-offs, and when to choose another way: {input}"
        ),
        "Alt text for the image" => format!(
            "Write alt text for the image described below — short, factual, saying what it shows for someone who cannot see it: {input}"
        ),
        _ => input.to_string(),
    }
}
