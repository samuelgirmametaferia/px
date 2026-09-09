// Cloudflare Worker + D1: the ONLY networked piece of the px registry.
// px clients POST scrubbed failure/unresolved reports here; nothing else.
// Reports never delete anything — the repair Action reads and decides.
//
// Endpoints (all auth: "Authorization: Bearer $REPORTS_TOKEN"):
//   POST /v1/report        { reports: [FailureReport...] }   from px clients
//   POST /v1/unresolved    { queries: [UnresolvedReport...] } from px clients
//   GET  /v1/failures      aggregated failure reports        for the repair Action
//   GET  /v1/unresolved    aggregated unresolved queries     for discovery
//
// No usernames, IPs, raw URLs, or raw queries are ever accepted or stored —
// clients send hashes and time buckets only, and the worker enforces that.

const REPORT_SQL = `INSERT INTO reports
  (record_id, method_id, registry_version, error_class,
   http_status, url_hash, ts_bucket, received_at)
  VALUES (?, ?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H','now'))`;

const UNRESOLVED_SQL = `INSERT INTO unresolved
  (query_hash, ts_bucket, received_at)
  VALUES (?, ?, strftime('%Y-%m-%dT%H','now'))`;

function unauthorized(env, request) {
  const auth = request.headers.get("authorization");
  return auth !== `Bearer ${env.REPORTS_TOKEN}`;
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    // ---- px clients: batched failure reports ------------------------------
    if (url.pathname === "/v1/report" && request.method === "POST") {
      if (unauthorized(env, request)) return new Response("forbidden", { status: 403 });
      const body = await request.json().catch(() => ({}));
      const reports = (body.reports || []).slice(0, 200); // batch cap
      if (reports.length === 0) return Response.json({ accepted: 0 });
      const stmts = reports.map((r) =>
        env.DB.prepare(REPORT_SQL).bind(
          String(r.record_id ?? "").slice(0, 200),
          String(r.method_id ?? "").slice(0, 100),
          Number(r.registry_version) || 0,
          String(r.error_class ?? "").slice(0, 40),
          r.http_status != null ? Number(r.http_status) : null,
          String(r.url_hash ?? "").slice(0, 64),
          String(r.timestamp_bucket ?? "").slice(0, 16),
        ),
      );
      await env.DB.batch(stmts);
      return Response.json({ accepted: reports.length });
    }

    // ---- px clients: unresolved queries (hashed, never raw) ---------------
    if (url.pathname === "/v1/unresolved" && request.method === "POST") {
      if (unauthorized(env, request)) return new Response("forbidden", { status: 403 });
      const body = await request.json().catch(() => ({}));
      const queries = (body.queries || []).slice(0, 200);
      if (queries.length === 0) return Response.json({ accepted: 0 });
      const stmts = queries.map((q) =>
        env.DB.prepare(UNRESOLVED_SQL).bind(
          String(q.query_hash ?? "").slice(0, 64),
          String(q.timestamp_bucket ?? "").slice(0, 16),
        ),
      );
      await env.DB.batch(stmts);
      return Response.json({ accepted: queries.length });
    }

    // ---- repair Action: aggregated failures -------------------------------
    if (url.pathname === "/v1/failures" && request.method === "GET") {
      if (unauthorized(env, request)) return new Response("forbidden", { status: 403 });
      // "48h" / "168h" / "720h" → SQLite modifier "-48 hours"
      const raw = url.searchParams.get("since") || "168h";
      const hours = parseInt(raw, 10);
      const modifier = Number.isFinite(hours) && hours > 0 ? `-${hours} hours` : "-168 hours";
      const { results } = await env.DB.prepare(
        `SELECT record_id, error_class, COUNT(*) as n, MAX(received_at) as last
         FROM reports
         WHERE received_at >= datetime('now', ?)
         GROUP BY record_id, error_class
         ORDER BY n DESC LIMIT 1000`,
      ).bind(modifier).all();
      return Response.json(results);
    }

    // ---- discovery: aggregated unresolved queries --------------------------
    if (url.pathname === "/v1/unresolved" && request.method === "GET") {
      if (unauthorized(env, request)) return new Response("forbidden", { status: 403 });
      const { results } = await env.DB.prepare(
        `SELECT query_hash, COUNT(*) as n, MAX(received_at) as last
         FROM unresolved
         WHERE received_at >= datetime('now', '-720 hours')
         GROUP BY query_hash
         ORDER BY n DESC LIMIT 500`,
      ).all();
      return Response.json(results);
    }

    return new Response("not found", { status: 404 });
  },
};
