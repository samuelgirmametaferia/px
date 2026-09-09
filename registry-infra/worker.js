// Cloudflare Worker + D1: the ONLY networked piece. px clients POST
// scrubbed failure reports here; nothing else. Reports never delete —
// the repair Action reads and decides.
export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/v1/report" && request.method === "POST") {
      const auth = request.headers.get("authorization");
      if (auth !== `Bearer ${env.REPORTS_TOKEN}`) {
        return new Response("forbidden", { status: 403 });
      }
      const body = await request.json();
      const reports = body.reports || [];
      const stmts = reports.map((r) => ({
        sql: `INSERT INTO reports
              (record_id, method_id, registry_version, error_class,
               http_status, url_hash, ts_bucket, received_at)
              VALUES (?, ?, ?, ?, ?, ?, ?, strftime('%Y-%m-%dT%H','now'))`,
        args: [r.record_id, r.method_id, r.registry_version, r.error_class,
               r.http_status ?? null, r.url_hash, r.timestamp_bucket],
      }));
      const batch = env.DB.batch(
        stmts.map((s) => env.DB.prepare(s.sql).bind(...s.args))
      );
      await batch;
      return Response.json({ accepted: reports.length });
    }
    // the repair Action reads aggregates (auth-gated)
    if (url.pathname === "/v1/failures" && request.method === "GET") {
      const auth = request.headers.get("authorization");
      if (auth !== `Bearer ${env.REPORTS_TOKEN}`) {
        return new Response("forbidden", { status: 403 });
      }
      const { results } = await env.DB.prepare(
        `SELECT record_id, error_class, COUNT(*) as n, MAX(received_at) as last
         FROM reports GROUP BY record_id, error_class ORDER BY n DESC LIMIT 1000`
      ).all();
      return Response.json(results);
    }
    return new Response("not found", { status: 404 });
  },
};
