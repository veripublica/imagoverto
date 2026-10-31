// imagoverto-wasm demo: the conversion runs here, off the page's thread.
// Messages in: { type: "init", pkg } then { type: "convert", bytes, options }.
// Messages out: { ok: true, ... } or { ok: false, kind, message }.
let wasm = null;

self.onmessage = async (e) => {
  const msg = e.data;
  try {
    if (msg.type === "init") {
      wasm = await import(msg.pkg);
      await wasm.default();
      self.postMessage({ ok: true, version: wasm.version() });
    } else if (msg.type === "convert") {
      const r = wasm.convert(msg.bytes, msg.options);
      self.postMessage({ ok: true, result: r }, [r.data.buffer]);
    }
  } catch (err) {
    // A thrown imagoverto error carries `kind`; a trap (RuntimeError) does not.
    self.postMessage({ ok: false, kind: err?.kind ?? "Internal", message: String(err?.message ?? err) });
  }
};
