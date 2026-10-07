// Syntax-check the explorer scripts and reject secret-shaped needles.
// This is not eslint. `npm run lint` is this script plus `tsc --noEmit`.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const scripts = readdirSync(join(root, "scripts")).filter((name) => name.endsWith(".mjs"));
for (const name of scripts) {
  const file = join(root, "scripts", name);
  const checked = spawnSync(process.execPath, ["--check", file], { encoding: "utf8" });
  if (checked.status !== 0) {
    process.stderr.write(checked.stderr || "");
    process.exit(checked.status || 1);
  }
}

// Pieces stay split so this detector is not itself a leak-scan hit.
const akia = "AK" + "IA";
const ghp = "gh" + "p_";
const live = "sk_" + "live_";
const begin = "BE" + "GIN";
const priv = "PRI" + "VATE";
const ssh = "OPEN" + "SSH";
const dashes = "-----";
const needles = [
  ["aws_access_key_prefix", new RegExp(akia + "[0-9A-Z]{16}")],
  ["github_pat_prefix", new RegExp(ghp + "[A-Za-z0-9]{20,}")],
  ["stripe_live_prefix", new RegExp(live + "[A-Za-z0-9]+")],
  ["BEGIN_OPENSSH", new RegExp(dashes + begin + " " + ssh + " " + priv + " KEY" + dashes)],
  ["BEGIN_PRIVATE", new RegExp(dashes + begin + " (?:RSA |EC |" + ssh + " )?" + priv + " KEY" + dashes)],
  ["tailnet_100_109", new RegExp("100" + "\\.109\\." + "\\d+\\.\\d+")],
];
const skip = new Set(["node_modules", ".wrangler", ".git"]);

function walk(dir) {
  const hits = [];
  for (const name of readdirSync(dir)) {
    if (skip.has(name) || name === "check-syntax.mjs") continue;
    const path = join(dir, name);
    const info = statSync(path);
    if (info.isDirectory()) {
      hits.push(...walk(path));
      continue;
    }
    if (info.size > 2_000_000) continue;
    let text;
    try {
      text = readFileSync(path);
    } catch {
      continue;
    }
    if (text.includes(0)) continue;
    const body = text.toString("utf8");
    for (const [label, pattern] of needles) {
      if (pattern.test(body)) hits.push(`${label} ${relative(root, path)}`);
    }
  }
  return hits;
}

const hits = walk(root);
if (hits.length) {
  process.stderr.write(hits.join("\n") + "\n");
  process.exit(1);
}
process.stdout.write(`syntax ok (${scripts.length} scripts)\n`);
