# 📘 eBook Prompt Suite

**eBook Prompt Suite** is a two-part system for generating clean, story-safe, **16:9 crop-safe image prompts and images** for illustrated eBooks.

It combines:

- ☁️ **Cloudflare Worker** (AI image generation + CORS-safe API)
- 🦀 **Yew (Rust → WASM)** frontend for prompt building, batch image generation, and post-processing

Designed for **static hosting (IIS / Hostek)** and modern AI workflows.

---

## ✨ Features

### Frontend (Yew)
- Premise-based prompt generation (Cover → Credits)
- Enforced **16:9 crop-safe composition**
- Strong **NO TEXT** prompt rules (prevents signage, labels, gibberish)
- Batch image generation
- Automatic **post-processing to 16:9 PNG** for downloads
- LocalStorage persistence:
  - Worker URL
  - API key
  - Premise text
- Static output via `trunk build` (no server required)

### Backend (Cloudflare Worker)
- Uses Cloudflare AI bindings
- Supports:
  - **FLUX** (fast, cinematic, animated-3D style)
  - **SDXL** (optional, more control)
- Hardened CORS handling (Origin allowlist)
- Optional API key protection
- Prompt length safety handling (≤2048 chars)
- Zero server state

---

## 📂 Repository Structure

```
ebook-prompt-suite/
├── worker/          # Cloudflare Worker (TypeScript)
│   ├── src/
│   │   └── index.ts
│   ├── wrangler.jsonc
│   └── package.json
│
├── yew/             # Yew frontend (Rust → WASM)
│   ├── src/
│   │   └── main.rs
│   ├── index.html
│   ├── Cargo.toml
│   ├── Trunk.toml
│   └── Cargo.lock
│
└── .gitignore
```

---

## 🚀 Getting Started

### 1️⃣ Cloudflare Worker

```bash
cd worker
npm install
npx wrangler dev
```

Optional production deploy:
```bash
npx wrangler deploy
```

Environment variables:
```
API_KEY=your_optional_key
ALLOWED_ORIGINS=https://www.webhtml5.info,http://localhost:8080
```

---

### 2️⃣ Yew Frontend (Local Dev)

```bash
cd yew
trunk serve --open
```

---

### 3️⃣ Production Build (Static Files)

```bash
cd yew
trunk build --release
```

Upload `yew/dist/` to:
```
/ebook-prompt-studio/
```

---

## 🌐 Deployment Notes (Hostek / IIS)

- Pure static files
- Ensure `.wasm` MIME type = `application/wasm`
- App URL:
  https://www.webhtml5.info/ebook-prompt-studio/

---

## 🔐 Security

- API keys never committed
- Secrets stored via Cloudflare or LocalStorage

---

## 🧠 Prompt Design

- No text in images
- Centered composition
- Family-safe visuals
- HTML overlays for final text

---

## 📌 Status

✅ Stable  
🚧 Active development

---

## 🧑‍💻 Author

Built by **MikeGyver / ekim5sg**
