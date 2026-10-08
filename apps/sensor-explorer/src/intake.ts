// Deterministic intake filter for MCP user input.
// No model call. A rejection names a category and never repeats the submitted text.

export type IntakeCategory = "prompt_injection" | "sexual_content" | "video" | "spam" | "oversized";

export type IntakeVerdict = { ok: true } | { ok: false; category: IntakeCategory };

const MAX_DEPTH = 16;
const MAX_KEYS = 200;
const MAX_STRINGS = 400;
const MAX_STRING = 8000;
const MAX_CHARS = 100_000;

const INJECTION = [
  /ignore\s+(?:all\s+|any\s+|the\s+)?(?:previous|prior|above|earlier)\s+(?:instructions|prompts|rules|directions)/,
  /disregard\s+(?:all\s+|any\s+|the\s+)?(?:previous|prior|above|system)/,
  /forget\s+(?:everything|all|your)\s+(?:previous|prior|instructions|rules)/,
  /you\s+are\s+now\s+(?:a|an|the|in)\b/,
  /\b(?:jailbreak|do\s+anything\s+now)\b/,
  /system\s+prompt/,
  /developer\s+mode/,
  /<\s*\/?\s*(?:system|im_start|inst)\b/,
  /\[\s*inst\s*\]/,
  /<<\s*sys\s*>>/,
  /reveal\s+(?:your|the)\s+(?:system\s+)?(?:prompt|instructions)/,
  /new\s+(?:system\s+)?instructions?\s*:/,
  /override\s+(?:the\s+)?(?:system|safety|previous)/,
];

const SEXUAL = [
  /\b(?:porn|porno|pornography|pornhub|xvideos|xhamster|redtube|onlyfans|hentai)\b/,
  /\b(?:nudes?|naked|nsfw|blowjob|handjob|sex\s*tape)\b/,
  /\bsex\b/,
  /\bxxx\b/,
];

const VIDEO =
  /data:\s*video\b|\.(?:mp4|webm|mov|mkv|avi|m4v|mpeg|mpg|wmv|flv)(?:\b|[\s?#]|$)|(?:youtube\.com|youtu\.be|vimeo\.com|dailymotion\.com|tiktok\.com)\b/;

const SPAM = [
  /<\s*script\b|javascript\s*:|data:\s*text\/html|onerror\s*=|onload\s*=/,
  /\b(?:viagra|cialis|casino|crypto\s+airdrop|free\s+money|seo\s+services)\b/,
  /\b(?:buy\s+now|click\s+here|limited\s+time\s+offer|act\s+now)\b/,
];

const HOMO: Record<string, string> = {
  "\u0430": "a",
  "\u0435": "e",
  "\u043e": "o",
  "\u0440": "p",
  "\u0441": "c",
  "\u0443": "y",
  "\u0445": "x",
  "\u0456": "i",
  "\u0455": "s",
};

type Budget = { strings: number; chars: number };

function fold(raw: string, leet: boolean): string {
  let s = raw.normalize("NFKC");
  s = s.replace(/[\u200b-\u200d\ufeff\u2060\u202a-\u202e\u2066-\u2069]/g, "");
  s = [...s].map((ch) => HOMO[ch] ?? ch).join("");
  s = s.toLowerCase();
  if (leet) s = s.replace(/[013457@$]/g, (ch) => ({ "0": "o", "1": "i", "3": "e", "4": "a", "5": "s", "7": "t", "@": "a", $: "s" })[ch] ?? ch);
  return s.replace(/\s+/g, " ");
}

function classify(raw: string): IntakeCategory | null {
  if (/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(raw)) return "spam";
  const plain = fold(raw, false);
  const leet = fold(raw, true);
  if (INJECTION.some((re) => re.test(plain) || re.test(leet))) return "prompt_injection";
  if (SEXUAL.some((re) => re.test(plain) || re.test(leet))) return "sexual_content";
  if (VIDEO.test(plain)) return "video";
  if (SPAM.some((re) => re.test(plain) || re.test(leet))) return "spam";
  if (/(.)\1{29,}/.test(plain)) return "spam";
  const urls = plain.match(/https?:\/\/\S+/g);
  if (urls && urls.length > 12) return "spam";
  return null;
}

function walk(value: unknown, depth: number, budget: Budget): IntakeVerdict {
  if (depth > MAX_DEPTH) return { ok: false, category: "oversized" };
  if (value == null || typeof value === "boolean" || typeof value === "number") return { ok: true };
  if (typeof value === "string") {
    budget.strings += 1;
    budget.chars += value.length;
    if (budget.strings > MAX_STRINGS || budget.chars > MAX_CHARS || value.length > MAX_STRING) {
      return { ok: false, category: "oversized" };
    }
    const category = classify(value);
    return category ? { ok: false, category } : { ok: true };
  }
  if (Array.isArray(value)) {
    if (value.length > MAX_STRINGS) return { ok: false, category: "oversized" };
    for (const item of value) {
      const verdict = walk(item, depth + 1, budget);
      if (!verdict.ok) return verdict;
    }
    return { ok: true };
  }
  if (typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>);
    if (entries.length > MAX_KEYS) return { ok: false, category: "oversized" };
    for (const [key, inner] of entries) {
      const keyVerdict = walk(key, depth + 1, budget);
      if (!keyVerdict.ok) return keyVerdict;
      const verdict = walk(inner, depth + 1, budget);
      if (!verdict.ok) return verdict;
    }
    return { ok: true };
  }
  return { ok: false, category: "spam" };
}

/** Screen one MCP arguments object. Fail closed. The verdict never contains the input. */
export function screenIntake(value: unknown): IntakeVerdict {
  try {
    return walk(value, 0, { strings: 0, chars: 0 });
  } catch {
    return { ok: false, category: "spam" };
  }
}

export function intakeRefusal(category: IntakeCategory) {
  return {
    content: [{
      type: "text",
      text: JSON.stringify({
        ok: false,
        status: "rejected",
        category,
        note: "Rejected by the intake filter. Nothing was stored.",
      }),
    }],
    isError: true as const,
  };
}
