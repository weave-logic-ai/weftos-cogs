/** Static guide for people and agents who want to use the Sensor Explorer MCP. */
export function mcpGuidePage(name: string): string {
  const title = escapeHtml(name);
  return `<!doctype html><html lang=en><head><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<meta name=color-scheme content="dark">
<title>MCP guide — ${title}</title>
<style>
:root{--bg:#08080A;--card:#16161C;--ink:#E0DEE8;--soft:#AAA8B4;--grey:#706E7A;--line:rgba(255,255,255,.094);--gold:#C4A25C;--green:#6EC896}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:16px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif}
a{color:var(--ink)}
a:hover{color:var(--gold)}
:focus-visible{outline:2px solid var(--gold);outline-offset:2px}
header,main{max-width:760px;margin:0 auto;padding:28px 20px}
header{padding-bottom:0}
h1{font-size:28px;line-height:1.2;margin:0 0 8px}
h2{font-size:18px;margin:28px 0 8px}
p,li{color:var(--soft)}
.kicker{color:var(--grey);font-size:12px;letter-spacing:.08em;text-transform:uppercase;font-weight:700}
code,pre{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:13px}
code{background:#202028;padding:1px 6px;border-radius:5px;color:var(--ink)}
pre{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:14px;overflow:auto;color:var(--ink);white-space:pre-wrap;overflow-wrap:anywhere}
table{width:100%;border-collapse:collapse;font-size:14px}
th,td{text-align:left;padding:8px 8px 8px 0;border-bottom:1px solid var(--line);vertical-align:top}
th{color:var(--grey);font-size:12px;letter-spacing:.04em;text-transform:uppercase}
.note{border-left:3px solid var(--gold);padding:8px 12px;background:var(--card);border-radius:0 8px 8px 0}
.ok{color:var(--green)}
@media(max-width:640px){h1{font-size:24px}header,main{padding:20px 16px}table,pre{font-size:12px}}
</style></head><body>
<header>
  <p class=kicker><a href="/">Catalog</a> · MCP</p>
  <h1>Add to the catalog from an agent, or from here.</h1>
  <p>The Sensor Explorer keeps a hardware catalog of projects, modules, and chips. A person can browse it. An agent can search it and propose a new part. A proposal stays pending until a person approves it.</p>
</header>
<main>
  <h2>Connect</h2>
  <p>Send JSON-RPC 2.0 to <code>POST /mcp</code> on this host. One request, one JSON response. This page is the GET on the same path, so a browser and an agent share the address.</p>
  <pre>POST /mcp
Authorization: Bearer &lt;api-key&gt;
Content-Type: application/json</pre>
  <p>An operator creates the key. A row in <code>api_keys</code> has <code>key</code>, <code>owner</code>, and <code>scope</code>. The worker secret <code>BOOTSTRAP_API_KEY</code> is an admin key. This page does not issue a key.</p>
  <table>
    <tr><th>Scope</th><th>What it can do</th></tr>
    <tr><td><code>read</code></td><td>Search, read a part, list cogs, and ask for the item schema.</td></tr>
    <tr><td><code>contribute</code></td><td>Everything a read key can do, plus submit a pending part.</td></tr>
    <tr><td><code>admin</code></td><td>The same writes as contribute.</td></tr>
  </table>

  <h2>What an agent can call</h2>
  <p>Start with <code>initialize</code>, then <code>tools/list</code>. The tools are <code>catalog_release</code>, <code>search_sensors</code>, <code>get_part</code>, <code>list_tree</code>, <code>suggest_for_task</code>, <code>search_pool</code>, <code>list_cogs</code>, <code>find_cog</code>, <code>promote_from_pool</code>, <code>add_part</code>, and <code>request_new_item</code>.</p>

  <h2>Propose a part</h2>
  <p class=note>Ask for the schema first. Fill one record. It is stored as a pending contribution. It does not appear in search until a person approves it.</p>
  <p><code>request_new_item</code> with <code>{"schema":true}</code> returns the whole item schema: a Project, a Module, or a Chip, including a buy link, a document link, and firmware facts. The field names match <code>crates/cog-market/src/hw.rs</code>. Leave <code>hash</code> empty. A reviewer sets it.</p>
  <pre>{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"request_new_item","arguments":{"schema":true}}}</pre>
  <p>Submit with a contribute or admin key. <code>id</code> is lowercase and hyphenated. <code>name</code> is required. <code>type</code> is <code>project</code>, <code>module</code>, or <code>chip</code>.</p>
  <pre>{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"request_new_item","arguments":{"type":"module","item":{"id":"example-probe","name":"Example probe","kind":"sensor","summary":"A bench probe over I2C."}}}}</pre>
  <p><code>add_part</code> and <code>promote_from_pool</code> use the same pending queue. A pool promotion needs the part's MPN.</p>

  <h2>What is refused</h2>
  <p>Every tool argument is screened before it is stored or used as a query. The check is a fixed set of rules. It refuses prompt injection, sexual content, video links (file types and common video hosts), spam, and oversized text. Send a datasheet URL when you have one. A PDF link is fine.</p>
  <p>A refusal looks like this, and it does not repeat what you sent:</p>
  <pre>{"ok":false,"status":"rejected","category":"prompt_injection","note":"Rejected by the intake filter. Nothing was stored."}</pre>
  <p class=ok>A clear sensor description is the thing to send. Name the part, the bus, and what it measures.</p>
  <p><a href="/">Back to the catalog</a></p>
</main>
</body></html>`;
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
