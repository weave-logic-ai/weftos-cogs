// Catalog attention: page opens, MCP reads, research notes, cog requests, contributions.
// Every write is best-effort. A database that has not applied 0006_activity.sql still serves the catalog.

import type { Env } from "./search";

export interface Activity {
  views: number;
  mcp_reads: number;
  last_at: string | null;
  events: any[];
  research: any[];
  cog_requests: any[];
  contributions: any[];
}

export interface Pulse {
  totals: { views: number; mcp_reads: number; research_queued: number; contributions_pending: number };
  recent: any[];
}

const EMPTY: Activity = { views: 0, mcp_reads: 0, last_at: null, events: [], research: [], cog_requests: [], contributions: [] };

function missing(e: unknown): boolean {
  return /no such table/i.test(String((e as { message?: string })?.message || e));
}

export async function recordEvent(db: Env["DB"], kind: string, refId: string, tool = "", note = "", refType = "part"): Promise<void> {
  try {
    await db.prepare(
      "INSERT INTO events (kind, tool, ref_type, ref_id, note, created) VALUES (?,?,?,?,?,?)"
    ).bind(kind, tool, refType, refId.slice(0, 180), note.slice(0, 240), new Date().toISOString()).run();
  } catch (e) {
    if (!missing(e)) console.error("recordEvent", e);
  }
}

/** Count a human open or an MCP get_part. Other MCP tools are events only. */
export async function recordLook(db: Env["DB"], refId: string, kind: "view" | "mcp"): Promise<void> {
  if (!refId) return;
  const now = new Date().toISOString();
  try {
    await db.prepare(
      "INSERT INTO events (kind, tool, ref_type, ref_id, note, created) VALUES (?,?,?,?,?,?)"
    ).bind(kind, kind === "mcp" ? "get_part" : "", "part", refId, "", now).run();
    const views = kind === "view" ? 1 : 0;
    const reads = kind === "mcp" ? 1 : 0;
    const col = kind === "view" ? "views" : "mcp_reads";
    await db.prepare(
      `INSERT INTO part_stats (ref_id, views, mcp_reads, last_at) VALUES (?, ?, ?, ?)
       ON CONFLICT(ref_id) DO UPDATE SET ${col} = ${col} + 1, last_at = excluded.last_at`
    ).bind(refId, views, reads, now).run();
  } catch (e) {
    if (!missing(e)) console.error("recordLook", e);
  }
}

export async function activityFor(db: Env["DB"], refId: string): Promise<Activity> {
  try {
    const stats = await db.prepare("SELECT views, mcp_reads, last_at FROM part_stats WHERE ref_id=?").bind(refId).first<{ views: number; mcp_reads: number; last_at: string }>();
    const events = await db.prepare("SELECT kind, tool, note, created FROM events WHERE ref_id=? ORDER BY id DESC LIMIT 12").bind(refId).all();
    const research = await db.prepare("SELECT note, status, created FROM research WHERE ref_id=? ORDER BY id DESC LIMIT 8").bind(refId).all();
    const requests = await db.prepare("SELECT note, status, created FROM cog_requests WHERE part_id=? ORDER BY id DESC LIMIT 8").bind(refId).all();
    const contributions = await db.prepare("SELECT type, author, status, created FROM contributions WHERE part_id=? ORDER BY id DESC LIMIT 8").bind(refId).all();
    return {
      views: stats?.views || 0,
      mcp_reads: stats?.mcp_reads || 0,
      last_at: stats?.last_at || null,
      events: events.results || [],
      research: research.results || [],
      cog_requests: requests.results || [],
      contributions: contributions.results || [],
    };
  } catch (e) {
    if (!missing(e)) console.error("activityFor", e);
    return EMPTY;
  }
}

export async function pulse(db: Env["DB"]): Promise<Pulse> {
  const totals = { views: 0, mcp_reads: 0, research_queued: 0, contributions_pending: 0 };
  let recent: any[] = [];
  try {
    const sums = await db.prepare("SELECT COALESCE(SUM(views),0) views, COALESCE(SUM(mcp_reads),0) mcp_reads FROM part_stats").first<{ views: number; mcp_reads: number }>();
    totals.views = sums?.views || 0;
    totals.mcp_reads = sums?.mcp_reads || 0;
  } catch (e) {
    if (!missing(e)) console.error("pulse stats", e);
  }
  try {
    const q = await db.prepare("SELECT COUNT(*) n FROM research WHERE status='queued'").first<{ n: number }>();
    totals.research_queued = q?.n || 0;
  } catch { /* research table is older; ignore */ }
  try {
    const q = await db.prepare("SELECT COUNT(*) n FROM contributions WHERE status='pending'").first<{ n: number }>();
    totals.contributions_pending = q?.n || 0;
  } catch { /* ignore */ }
  try {
    const rows = await db.prepare("SELECT kind, tool, ref_id, note, created FROM events ORDER BY id DESC LIMIT 8").all();
    recent = rows.results || [];
  } catch (e) {
    if (!missing(e)) console.error("pulse events", e);
  }
  return { totals, recent };
}
