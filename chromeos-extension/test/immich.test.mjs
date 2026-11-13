// Run: node --test chromeos-extension/test/
import assert from "node:assert/strict";
import { test } from "node:test";
import { ImmichClient, UnauthorizedError, parseShareLink, pick } from "../immich.js";

/** Fake fetch routing on pathname (+ optional query predicate). */
function fakeFetch(routes) {
  return async (url) => {
    const u = new URL(url);
    assert.equal(u.searchParams.get("key"), "k1", "share key must be sent");
    for (const [p, when, status, body, type = "application/json"] of routes) {
      if (u.pathname === p && when(u.searchParams)) {
        const payload = typeof body === "string" ? body : JSON.stringify(body);
        return new Response(payload, { status, headers: { "content-type": type } });
      }
    }
    return new Response("not found", { status: 404 });
  };
}
const any = () => true;
const link = parseShareLink("https://photos.example.com/share/k1");
const me = { album: { id: "al", assets: [] }, assets: [] };

test("parses share links", () => {
  assert.deepEqual(parseShareLink(" https://p.example.com:8443/s/fam/photos/1 "), {
    server: "https://p.example.com:8443",
    authParam: "slug",
    authValue: "fam",
  });
  assert.throws(() => parseShareLink("https://p.example.com/albums/1"));
});

test("lists via album endpoint", async () => {
  const c = new ImmichClient(link, fakeFetch([
    ["/api/shared-links/me", any, 200, me],
    ["/api/albums/al", any, 200, { assets: [
      { id: "a", type: "IMAGE", exifInfo: { exifImageWidth: 3000, exifImageHeight: 4000 } },
      { id: "v", type: "VIDEO" },
    ] }],
  ]));
  const list = await c.listAssets();
  assert.deepEqual(list.map((a) => [a.id, a.isImage]), [["a", true], ["v", false]]);
  assert.equal(list[0].ratio, 0.75);
});

test("falls back to columnar timeline, retrying with size=MONTH", async () => {
  const c = new ImmichClient(link, fakeFetch([
    ["/api/shared-links/me", any, 200, me],
    ["/api/albums/al", any, 200, { id: "al" }],
    ["/api/timeline/buckets", (q) => q.get("size") === "MONTH", 200, [{ timeBucket: "2025-01-01" }]],
    ["/api/timeline/buckets", any, 400, {}],
    ["/api/timeline/bucket", (q) => q.get("size") === "MONTH", 200, { id: ["t1", "t2"], isImage: [true, false], ratio: [1.5, 1] }],
  ]));
  assert.deepEqual((await c.listAssets()).map((a) => a.id), ["t1", "t2"]);
});

test("revoked link → UnauthorizedError", async () => {
  const c = new ImmichClient(link, fakeFetch([["/api/shared-links/me", any, 401, {}]]));
  await assert.rejects(c.listAssets(), UnauthorizedError);
});

test("download falls back to preview when original is forbidden", async () => {
  const c = new ImmichClient(link, fakeFetch([
    ["/api/assets/x/original", any, 403, ""],
    ["/api/assets/x/thumbnail", (q) => q.get("size") === "preview", 200, "PREV", "image/jpeg"],
  ]));
  const img = await c.download({ id: "x", isImage: true, mime: "image/jpeg" }, true);
  assert.equal(Buffer.from(img.bytes).toString(), "PREV");
});

test("pick avoids recent repeats and honors orientation", () => {
  const assets = Array.from({ length: 10 }, (_, i) => ({ id: String(i), isImage: true, ratio: 1.5 }));
  assets.push({ id: "tall", isImage: true, ratio: 0.5 });
  const history = [];
  const seq = Array.from({ length: 300 }, () => pick(assets, "landscape", history).id);
  assert.ok(!seq.includes("tall"));
  for (let i = 0; i + 6 <= seq.length; i++) {
    assert.equal(new Set(seq.slice(i, i + 6)).size, 6);
  }
  assert.equal(pick([], "any", []), null);
});
