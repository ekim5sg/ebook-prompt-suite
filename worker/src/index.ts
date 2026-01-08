export interface Env {
  AI: Ai;
  API_KEY?: string;

  // Single origin convenience (optional)
  ALLOWED_ORIGIN?: string;

  // Comma-separated allowlist (recommended)
  // Example:
  // "http://localhost:8080,https://www.webhtml5.info,https://webhtml5.info"
  ALLOWED_ORIGINS?: string;
}

const MAX_PROMPT_CHARS = 2048;

/**
 * Trim to the model limit, but keep the *end* of the prompt,
 * because the style / anti-text / safety lines are appended at the end
 * and are usually the most important instructions.
 */
function trimToMaxPrompt(s: string, maxChars = MAX_PROMPT_CHARS): string {
  const t = (s ?? "").trim();
  if (t.length <= maxChars) return t;
  return t.slice(t.length - maxChars);
}

function parseAllowedOrigins(env: Env): Set<string> {
  const s = new Set<string>([
    "http://localhost:8080",
    "http://127.0.0.1:8080",
    "http://[::1]:8080",
    // Hostek production (you said you always use www, but keeping both is fine)
    "https://www.webhtml5.info",
    "https://webhtml5.info",
  ]);

  if (env.ALLOWED_ORIGIN?.trim()) s.add(env.ALLOWED_ORIGIN.trim());

  if (env.ALLOWED_ORIGINS?.trim()) {
    for (const part of env.ALLOWED_ORIGINS.split(",")) {
      const o = part.trim();
      if (o) s.add(o);
    }
  }

  return s;
}

function corsHeaders(request: Request, env: Env): Record<string, string> {
  const origin = request.headers.get("Origin") || "";
  const allowed = parseAllowedOrigins(env);

  const headers: Record<string, string> = {
    "Vary": "Origin",
    "Access-Control-Allow-Methods": "POST, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, Authorization",
    "Access-Control-Max-Age": "86400",
    // Lets your browser JS read these response headers (optional but handy)
    "Access-Control-Expose-Headers": "X-Model, X-Steps, X-Style, X-Num-Steps, X-Guidance, X-Prompt-Chars",
  };

  // Only add Allow-Origin when this is an allowed browser origin.
  // (Never set it to an empty string.)
  if (origin && allowed.has(origin)) {
    headers["Access-Control-Allow-Origin"] = origin;
  }

  return headers;
}

function respond(
  request: Request,
  env: Env,
  body: BodyInit | null,
  init: ResponseInit & { headers?: Record<string, string> | Headers }
) {
  const headers = new Headers(init.headers);
  const ch = corsHeaders(request, env);
  for (const [k, v] of Object.entries(ch)) headers.set(k, v);
  return new Response(body, { ...init, headers });
}

/**
 * Stronger anti-text prompting:
 * - FLUX has no negative_prompt, so we must push text suppression into prompt.
 * - Also bans common "text carriers" (signs, posters, packaging, book spines, screens).
 * - Adds crop-safe 16:9 composition guidance for your eBook layouts.
 *
 * NOTE: Keep this under MAX_PROMPT_CHARS after all lines are combined.
 */
function buildStyledPrompt(userPrompt: string, style: string) {
  // Hard anti-text instruction (FLUX needs this in the prompt)
  const antiText =
    "ABSOLUTELY NO TEXT: no letters, no words, no numbers, no symbols, no signage, no labels, no captions, " +
    "no book covers with titles, no misspellings, no gibberish. " +
    "If any sign, poster, menu, label, packaging, screen, or book spine appears, it must be BLANK and UNREADABLE. " +
    "No logos, no watermark, no signature.";

  // Extra suppression: avoid common text-bearing props entirely
  const avoidTextProps =
    "Avoid text-bearing elements: posters, banners, street signs, storefront signs, menus, UI screens, labels, packaging, " +
    "newspapers, magazines, chalkboards, whiteboards, license plates, book spines with titles. Prefer plain surfaces and simple shapes.";

  // Composition for downstream crop + consistent eBook layout
  const cropSafe =
    "Keep key subjects centered with generous margins; avoid important details near edges (crop-safe 16:9).";

  const baseSafety =
    "Family-friendly. Natural proportions. No gore. No violence. No weapons. No horror imagery.";

  const storybook =
    "Storybook illustration, warm cinematic lighting, clean composition, soft depth of field, high detail.";

  const animated3d =
    "High-quality 3D animated family film look, soft global illumination, warm rim light, detailed materials, subtle subsurface scattering, " +
    "clean shapes, cinematic depth of field, sharp focus on subject, ultra clean render.";

  const styleLine = style === "animated3d" ? animated3d : storybook;

  // Build full prompt then trim to max chars (keeping the end).
  const full = `${userPrompt.trim()}

${cropSafe}
${styleLine}
${antiText}
${avoidTextProps}
${baseSafety}`;

  return trimToMaxPrompt(full);
}

function clampInt(n: unknown, min: number, max: number, fallback: number) {
  const v = typeof n === "number" && Number.isFinite(n) ? Math.round(n) : fallback;
  return Math.min(max, Math.max(min, v));
}

function clampNum(n: unknown, min: number, max: number, fallback: number) {
  const v = typeof n === "number" && Number.isFinite(n) ? n : fallback;
  return Math.min(max, Math.max(min, v));
}

type GenerateRequest = {
  prompt: string;
  model?: "flux" | "sdxl";
  style?: "storybook" | "animated3d";
  steps?: number; // flux
  seed?: number;

  // sdxl
  width?: number;
  height?: number;
  num_steps?: number;
  guidance?: number;
  negative_prompt?: string;
};

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);

    if (url.pathname === "/__version") {
      return respond(
        request,
        env,
        "ebook-image-forge v5 (prompt<=2048 + resilient CORS + stronger no-text)",
        { status: 200 }
      );
    }

    // ✅ Always answer OPTIONS cleanly with CORS headers
    if (request.method === "OPTIONS") {
      return respond(request, env, null, { status: 204 });
    }

    if (url.pathname !== "/api/generate") {
      return respond(request, env, "Not found", { status: 404 });
    }

    if (request.method !== "POST") {
      return respond(request, env, "Method not allowed", { status: 405 });
    }

    // Optional API key auth
    if (env.API_KEY) {
      const auth = request.headers.get("Authorization") || "";
      const token = auth.startsWith("Bearer ") ? auth.slice(7) : "";
      if (token !== env.API_KEY) {
        // IMPORTANT: still returns CORS headers (respond()) so browser doesn't misreport as CORS
        return respond(request, env, "Unauthorized", { status: 401 });
      }
    }

    let body: GenerateRequest;
    try {
      body = await request.json();
    } catch {
      return respond(request, env, "Invalid JSON", { status: 400 });
    }

    if (!body.prompt || body.prompt.trim().length < 3) {
      return respond(request, env, "prompt is required", { status: 400 });
    }

    const model = body.model ?? "flux";
    const style = body.style ?? "storybook";
    const prompt = buildStyledPrompt(body.prompt, style);

    // ----------------------------
    // FLUX (fast + animated3d vibe)
    // ----------------------------
    if (model === "flux") {
      const steps = clampInt(body.steps, 1, 8, 6);

      try {
        const response: any = await env.AI.run("@cf/black-forest-labs/flux-1-schnell", {
          prompt,
          steps,
          ...(typeof body.seed === "number" ? { seed: Math.floor(body.seed) } : {}),
        });

        const b64 = response?.image;
        if (!b64 || typeof b64 !== "string") {
          return respond(request, env, "Model returned no image", { status: 502 });
        }

        // Decode base64 → bytes
        const binaryString = atob(b64);
        const bytes = Uint8Array.from(binaryString, (m) => m.codePointAt(0) || 0);

        return respond(request, env, bytes, {
          status: 200,
          headers: {
            "Content-Type": "image/jpeg",
            "Cache-Control": "no-store",
            "X-Model": "flux-1-schnell",
            "X-Steps": String(steps),
            "X-Style": style,
            "X-Prompt-Chars": String(prompt.length),
          },
        });
      } catch (e: any) {
        // IMPORTANT: still returns CORS headers (respond()) so browser doesn't misreport as CORS
        const msg = (e && (e.message || String(e))) ? String(e.message || e) : "AI run error";
        return respond(request, env, `Upstream AI error: ${msg}`, { status: 502 });
      }
    }

    // ----------------------------
    // SDXL (better controllability)
    // ----------------------------
    const width = clampInt(body.width, 256, 2048, 1344);
    const height = clampInt(body.height, 256, 2048, 768);
    const num_steps = clampInt(body.num_steps, 1, 20, 20);
    const guidance = clampNum(body.guidance, 1, 20, 7.5);

    // Strong negative prompt to reduce text/typos
    const strongNoTextNegative =
      "text, letters, words, typography, caption, subtitle, title, logo, watermark, signature, " +
      "signage, street sign, label, menu, poster, banner, book cover text, misspelling, gibberish, " +
      "license plate, UI, screen text, packaging text, " +
      "low quality, blurry, grain, noise, deformed, malformed hands, extra fingers";

    const inputs = {
      prompt,
      negative_prompt: body.negative_prompt
        ? `${body.negative_prompt}, ${strongNoTextNegative}`
        : strongNoTextNegative,
      width,
      height,
      num_steps,
      guidance,
      ...(typeof body.seed === "number" ? { seed: Math.floor(body.seed) } : {}),
    };

    try {
      const imgBytes: any = await env.AI.run("@cf/stabilityai/stable-diffusion-xl-base-1.0", inputs);

      return respond(request, env, imgBytes as any, {
        status: 200,
        headers: {
          "Content-Type": "image/jpeg",
          "Cache-Control": "no-store",
          "X-Model": "sdxl-base-1.0",
          "X-Num-Steps": String(num_steps),
          "X-Guidance": String(guidance),
          "X-Style": style,
          "X-Prompt-Chars": String(prompt.length),
        },
      });
    } catch (e: any) {
      const msg = (e && (e.message || String(e))) ? String(e.message || e) : "AI run error";
      return respond(request, env, `Upstream AI error: ${msg}`, { status: 502 });
    }
  },
} satisfies ExportedHandler<Env>;
