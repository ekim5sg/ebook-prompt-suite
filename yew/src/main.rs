use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{
    Blob, BlobPropertyBag, CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement, Url,
};
use yew::prelude::*;

// ----------------------------
// LocalStorage helpers
// ----------------------------
const LS_WORKER_URL: &str = "ebook_prompt_studio_worker_url";
const LS_API_KEY: &str = "ebook_prompt_studio_api_key";
const LS_PREMISE: &str = "ebook_prompt_studio_premise";

fn load_local_storage(key: &str) -> String {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(key).ok().flatten())
        .unwrap_or_default()
}

fn save_local_storage(key: &str, value: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item(key, value);
    }
}

fn remove_local_storage(key: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.remove_item(key);
    }
}

// ----------------------------
// Prompt size control (Cloudflare AI limit)
// ----------------------------
const MAX_WORKER_PROMPT_CHARS: usize = 2048;

// ----------------------------
// App Models
// ----------------------------
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct PromptItem {
    key: String,      // "cover", "prologue", "ch1"... "credits"
    filename: String, // "cover.jpg"...
    prompt: String,
}

#[derive(Clone, Debug, PartialEq)]
struct RenderedImage {
    key: String,
    preview_filename: String,  // original worker output (jpg)
    preview_url: String,       // object URL for preview
    download_filename: String, // 16:9 png filename
    download_url: String,      // object URL for download
}

#[derive(Serialize)]
struct GenerateReq<'a> {
    prompt: &'a str,
    model: &'a str, // "flux"
    style: &'a str, // "animated3d"
    steps: u32,     // flux: max 8
    seed: Option<u32>,
}

// ----------------------------
// Prompt builder
// ----------------------------
fn trim_to_max_prompt(s: String, max_chars: usize) -> String {
    let t = s.trim().to_string();
    if t.chars().count() <= max_chars {
        return t;
    }
    // Keep the end of the prompt (style/safety lines are usually at the end)
    let mut out = String::new();
    let mut count = 0usize;
    for ch in t.chars().rev() {
        if count >= max_chars {
            break;
        }
        out.push(ch);
        count += 1;
    }
    out.chars().rev().collect::<String>()
}

fn build_prompt(premise: &str, slot: &str) -> String {
    let base = format!(
        "Illustrated eBook scene for: \"{premise}\". \
         Create a clean, family-friendly, storybook-cinematic image. \
         Landscape orientation. Compose for a 16:9 wide cinematic frame (safe to crop). \
         No text, no logos, no watermark."
    );

    let crop_safe =
        "Keep key subjects centered with generous margins; avoid important details near edges (crop-safe 16:9).";

    let slot_specific = match slot {
        "cover" => "Cover art: iconic moment that communicates the theme, clear focal subject, inviting warm lighting.",
        "prologue" => "Prologue scene: establish setting and mood, gentle intrigue, readable composition.",
        "ch1" => "Chapter 1 scene: introduce protagonist doing a simple action that sets the story in motion.",
        "ch2" => "Chapter 2 scene: friendly interaction or small challenge, upbeat tone.",
        "ch3" => "Chapter 3 scene: discovery moment—visual clue, mild suspense without fear.",
        "ch4" => "Chapter 4 scene: obstacle moment—show problem visually, still kid-safe.",
        "ch5" => "Chapter 5 scene: teamwork or learning moment—progress and hope.",
        "ch6" => "Chapter 6 scene: resolution moment—celebration or calm victory.",
        "epilogue" => "Epilogue scene: peaceful wrap-up, cozy closing image.",
        "credits" => "Credits background: simple pleasing backdrop with space for overlay later (but generate with NO TEXT).",
        _ => "Scene: cohesive with the story.",
    };

    // Safe “Pixar-adjacent” vibe without naming a specific studio.
    let animated_3d = "High-quality 3D animated family film look, soft global illumination, warm cinematic lighting, \
                      detailed materials, subtle subsurface scattering, clean shapes, crisp focus on subject, \
                      gentle depth of field, ultra clean render.";

    let full = format!(
        "{base} {crop_safe} {slot_specific} {animated_3d} Natural proportions."
    );

    // Ensure we do not exceed the worker prompt limit
    trim_to_max_prompt(full, MAX_WORKER_PROMPT_CHARS)
}

fn pretty_slot_name(key: &str) -> &str {
    match key {
        "cover" => "Cover",
        "prologue" => "Prologue",
        "ch1" => "Chapter 1",
        "ch2" => "Chapter 2",
        "ch3" => "Chapter 3",
        "ch4" => "Chapter 4",
        "ch5" => "Chapter 5",
        "ch6" => "Chapter 6",
        "epilogue" => "Epilogue",
        "credits" => "Credits",
        _ => key,
    }
}

// ----------------------------
// Helpers: bytes -> Blob -> object URL
// ----------------------------
fn bytes_to_object_url(bytes: &[u8], mime: &str) -> Result<String, String> {
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());

    let bag = BlobPropertyBag::new();
    bag.set_type(mime); // ✅ non-deprecated

    let blob = Blob::new_with_buffer_source_sequence_and_options(&parts, &bag)
        .map_err(|_| "Failed to create Blob")?;

    Url::create_object_url_with_blob(&blob).map_err(|_| "Failed to create object URL".to_string())
}

// ----------------------------
// 16:9 crop+resize -> PNG object URL
// ----------------------------
async fn make_16x9_png_object_url(
    preview_url: &str,
    out_w: u32,
    out_h: u32,
) -> Result<String, String> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or("No document")?;

    let img: HtmlImageElement = document
        .create_element("img")
        .map_err(|_| "create_element img failed")?
        .dyn_into()
        .map_err(|_| "dyn_into HtmlImageElement failed")?;

    // Wait for onload via oneshot
    let (tx, rx) = futures_channel::oneshot::channel::<Result<(), String>>();
    let tx = Rc::new(RefCell::new(Some(tx)));

    let tx2 = tx.clone();
    let onload = Closure::<dyn FnMut()>::new(move || {
        if let Some(sender) = tx2.borrow_mut().take() {
            let _ = sender.send(Ok(()));
        }
    });

    let tx3 = tx.clone();
    let onerror = Closure::<dyn FnMut()>::new(move || {
        if let Some(sender) = tx3.borrow_mut().take() {
            let _ = sender.send(Err("Image failed to load".to_string()));
        }
    });

    img.set_onload(Some(onload.as_ref().unchecked_ref()));
    img.set_onerror(Some(onerror.as_ref().unchecked_ref()));
    img.set_src(preview_url);

    onload.forget();
    onerror.forget();

    match rx.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("Image load channel canceled".to_string()),
    }

    let iw = img.natural_width() as f64;
    let ih = img.natural_height() as f64;
    if iw < 2.0 || ih < 2.0 {
        return Err("Invalid natural image size".to_string());
    }

    // Compute 16:9 crop rect
    let target_ratio = 16.0 / 9.0;
    let src_ratio = iw / ih;

    let (sx, sy, sw, sh) = if src_ratio > target_ratio {
        // too wide -> crop width
        let new_w = ih * target_ratio;
        let x = (iw - new_w) / 2.0;
        (x, 0.0, new_w, ih)
    } else {
        // too tall -> crop height
        let new_h = iw / target_ratio;
        let y = (ih - new_h) / 2.0;
        (0.0, y, iw, new_h)
    };

    // Canvas
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .map_err(|_| "create_element canvas failed")?
        .dyn_into()
        .map_err(|_| "dyn_into HtmlCanvasElement failed")?;

    canvas.set_width(out_w);
    canvas.set_height(out_h);

    let ctx: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(|_| "get_context failed")?
        .ok_or("2d context missing")?
        .dyn_into()
        .map_err(|_| "dyn_into CanvasRenderingContext2d failed")?;

    // IMPORTANT:
    // Some web-sys builds don't expose the 9-arg drawImage overload.
    // We crop using a transform instead:
    //
    // Map crop rect (sx,sy,sw,sh) -> canvas (0,0,out_w,out_h)
    let scale_x = out_w as f64 / sw;
    let scale_y = out_h as f64 / sh;

    // After scaling, translate by (-sx, -sy) so crop origin becomes (0,0)
    ctx.set_transform(scale_x, 0.0, 0.0, scale_y, -sx * scale_x, -sy * scale_y)
        .map_err(|_| "set_transform failed")?;

    // Draw full image; transform makes it behave like a cropped draw
    ctx.draw_image_with_html_image_element(&img, 0.0, 0.0)
        .map_err(|_| "draw_image failed")?;

    // Reset transform
    ctx.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0)
        .map_err(|_| "reset transform failed")?;

    // canvas -> PNG blob (FnMut-safe sender)
    let (txb, rxb) = futures_channel::oneshot::channel::<Result<Blob, String>>();
    let txb = Rc::new(RefCell::new(Some(txb)));

    let txb2 = txb.clone();
    let cb = Closure::<dyn FnMut(Option<Blob>)>::new(move |blob: Option<Blob>| {
        if let Some(sender) = txb2.borrow_mut().take() {
            if let Some(b) = blob {
                let _ = sender.send(Ok(b));
            } else {
                let _ = sender.send(Err("canvas.to_blob returned null".to_string()));
            }
        }
    });

    canvas
        .to_blob(cb.as_ref().unchecked_ref())
        .map_err(|_| "to_blob failed")?;
    cb.forget();

    let blob = match rxb.await {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("to_blob channel canceled".to_string()),
    };

    Url::create_object_url_with_blob(&blob).map_err(|_| "Failed to create PNG object URL".to_string())
}

// ----------------------------
// Yew App
// ----------------------------
#[function_component(App)]
fn app() -> Html {
    // Load initial values from LocalStorage (paste once, remember)
    let premise = use_state(|| load_local_storage(LS_PREMISE));

    let worker_url = use_state(|| {
        let v = load_local_storage(LS_WORKER_URL);
        if v.trim().is_empty() {
            "https://ebook-image-forge.mikegyver.workers.dev/api/generate".to_string()
        } else {
            v
        }
    });

    let api_key = use_state(|| load_local_storage(LS_API_KEY));

    let prompts = {
        let premise = premise.clone();
        use_state(move || {
            let keys = [
                "cover", "prologue", "ch1", "ch2", "ch3", "ch4", "ch5", "ch6", "epilogue", "credits",
            ];
            keys.iter()
                .map(|k| PromptItem {
                    key: k.to_string(),
                    filename: format!("{k}.jpg"),
                    prompt: build_prompt(&premise, k),
                })
                .collect::<Vec<_>>()
        })
    };

    let images = use_state(|| Vec::<RenderedImage>::new());
    let busy = use_state(|| false);
    let status = use_state(|| String::new());

    let regen_prompts = {
        let premise = premise.clone();
        let prompts = prompts.clone();
        Callback::from(move |_| {
            let prem = (*premise).clone();
            let keys = [
                "cover", "prologue", "ch1", "ch2", "ch3", "ch4", "ch5", "ch6", "epilogue", "credits",
            ];
            let next = keys
                .iter()
                .map(|k| PromptItem {
                    key: k.to_string(),
                    filename: format!("{k}.jpg"),
                    prompt: build_prompt(&prem, k),
                })
                .collect::<Vec<_>>();
            prompts.set(next);
        })
    };

    let clear_saved_key = {
        let api_key = api_key.clone();
        Callback::from(move |_| {
            remove_local_storage(LS_API_KEY);
            api_key.set(String::new());
        })
    };

    let on_generate_all = {
        let prompts = prompts.clone();
        let images = images.clone();
        let worker_url = worker_url.clone();
        let api_key = api_key.clone();
        let busy = busy.clone();
        let status = status.clone();

        Callback::from(move |_| {
            if *busy {
                return;
            }

            busy.set(true);
            images.set(vec![]);
            status.set("Generating images…".to_string());

            let prompts_list = (*prompts).clone();
            let url = (*worker_url).clone();
            let token = (*api_key).clone();
            let images_setter = images.clone();
            let busy_setter = busy.clone();
            let status_setter = status.clone();

            wasm_bindgen_futures::spawn_local(async move {
                let mut out: Vec<RenderedImage> = vec![];

                for (idx, item) in prompts_list.iter().enumerate() {
                    status_setter.set(format!(
                        "Generating {} ({}/{})…",
                        pretty_slot_name(&item.key),
                        idx + 1,
                        prompts_list.len()
                    ));

                    let req = GenerateReq {
                        prompt: &item.prompt,
                        model: "flux",
                        style: "animated3d",
                        steps: 8,
                        seed: None,
                    };

                    let mut r = Request::post(&url).header("Content-Type", "application/json");
                    if !token.trim().is_empty() {
                        r = r.header("Authorization", &format!("Bearer {}", token.trim()));
                    }

                    let resp = match r.json(&req).unwrap().send().await {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    if !resp.ok() {
                        // Read text error if present; helps debug without "CORS" confusion
                        let msg = resp.text().await.unwrap_or_else(|_| "Request failed".into());
                        status_setter.set(format!(
                            "{} failed: HTTP {} — {}",
                            pretty_slot_name(&item.key),
                            resp.status(),
                            msg
                        ));
                        continue;
                    }

                    let bytes = match resp.binary().await {
                        Ok(b) => b,
                        Err(_) => continue,
                    };

                    // Preview URL (JPEG)
                    let preview_url = match bytes_to_object_url(&bytes, "image/jpeg") {
                        Ok(u) => u,
                        Err(_) => continue,
                    };

                    // 16:9 PNG download (1600x900)
                    let png_url = match make_16x9_png_object_url(&preview_url, 1600, 900).await {
                        Ok(u) => u,
                        Err(_) => preview_url.clone(), // fallback
                    };

                    out.push(RenderedImage {
                        key: item.key.clone(),
                        preview_filename: item.filename.clone(),
                        preview_url,
                        download_filename: format!("{}.png", item.key),
                        download_url: png_url,
                    });

                    images_setter.set(out.clone());
                }

                status_setter.set("Done ✅".to_string());
                busy_setter.set(false);
            });
        })
    };

    html! {
        <div style="font-family: system-ui; max-width: 1100px; margin: 0 auto; padding: 16px;">
            <h1>{"eBook Prompt Studio → Cloudflare AI (FLUX) → Images"}</h1>

            if !(*status).is_empty() {
                <p style="opacity:0.85;">{(*status).clone()}</p>
            }

            <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 12px;">
                <div>
                    <label>{"eBook premise"}</label>
                    <textarea
                        style="width: 100%; height: 110px;"
                        value={(*premise).clone()}
                        oninput={{
                            let premise = premise.clone();
                            Callback::from(move |e: InputEvent| {
                                let v = e.target_unchecked_into::<web_sys::HtmlTextAreaElement>().value();
                                save_local_storage(LS_PREMISE, &v);
                                premise.set(v);
                            })
                        }}
                    />
                    <div style="display:flex; gap: 8px; margin-top: 8px;">
                        <button onclick={regen_prompts.clone()} disabled={*busy}>{"Regenerate prompts"}</button>
                        <button onclick={on_generate_all.clone()} disabled={*busy}>{"Generate images (batch)"}</button>
                    </div>

                    <p style="opacity:0.75; margin-top: 10px;">
                        {format!("Note: prompts are auto-trimmed to {} chars to match your Worker / Cloudflare AI limits.", MAX_WORKER_PROMPT_CHARS)}
                    </p>
                </div>

                <div>
                    <label>{"Worker URL"}</label>
                    <input
                        style="width: 100%;"
                        value={(*worker_url).clone()}
                        oninput={{
                            let worker_url = worker_url.clone();
                            Callback::from(move |e: InputEvent| {
                                let v = e.target_unchecked_into::<web_sys::HtmlInputElement>().value();
                                save_local_storage(LS_WORKER_URL, &v);
                                worker_url.set(v);
                            })
                        }}
                    />
                    <label style="display:block; margin-top: 8px;">{"App API Key (optional)"}</label>
                    <input
                        type="password"
                        style="width: 100%;"
                        value={(*api_key).clone()}
                        placeholder="Bearer token used by your Worker (not OpenAI)"
                        oninput={{
                            let api_key = api_key.clone();
                            Callback::from(move |e: InputEvent| {
                                let v = e.target_unchecked_into::<web_sys::HtmlInputElement>().value();
                                save_local_storage(LS_API_KEY, &v);
                                api_key.set(v);
                            })
                        }}
                    />
                    <div style="display:flex; gap: 8px; margin-top: 8px;">
                        <button onclick={clear_saved_key} disabled={*busy}>{"Clear saved key"}</button>
                    </div>
                    <p style="opacity:0.8; margin-top: 10px;">
                        {"Download links are 16:9 PNGs (post-processed). Preview is the original JPEG."}
                    </p>
                </div>
            </div>

            <hr />

            <h2>{"Prompts"}</h2>
            <div style="display: grid; grid-template-columns: 1fr; gap: 10px;">
                { for (*prompts).iter().map(|p| {
                    let title = format!("{} • {}", pretty_slot_name(&p.key), p.filename);
                    html!{
                        <div style="border: 1px solid #ddd; border-radius: 10px; padding: 10px;">
                            <div style="display:flex; justify-content: space-between; gap: 10px;">
                                <b>{title}</b>
                            </div>
                            <textarea style="width: 100%; height: 90px;" value={p.prompt.clone()} readonly=true />
                        </div>
                    }
                }) }
            </div>

            <hr />

            <h2>{"Generated Images"}</h2>
            if *busy {
                <p>{"Generating… (one request per image, then 16:9 PNG conversion)"}</p>
            }

            <div style="display: grid; grid-template-columns: repeat(2, 1fr); gap: 12px;">
                { for (*images).iter().map(|img| {
                    let title = format!("{} • {}", pretty_slot_name(&img.key), img.preview_filename);

                    let preview_href = img.preview_url.clone();
                    let preview_fn = img.preview_filename.clone();

                    let dl_href = img.download_url.clone();
                    let dl_fn = img.download_filename.clone();

                    html!{
                        <div style="border:1px solid #ddd; border-radius: 10px; padding: 10px;">
                            <b>{title}</b>
                            <img src={img.preview_url.clone()} style="width: 100%; border-radius: 8px; margin-top: 8px;" />

                            <div style="display:flex; gap: 12px; margin-top: 10px; flex-wrap: wrap;">
                                <a href={preview_href} download={preview_fn}>{"Download original (JPG)"}</a>
                                <a style="font-weight: 600;" href={dl_href} download={dl_fn}>{"Download 16:9 (PNG)"} </a>
                            </div>
                        </div>
                    }
                }) }
            </div>
        </div>
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}
