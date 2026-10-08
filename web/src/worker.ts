import init, { trace_rgba } from "./wasm/tracerva_wasm";
import wasmUrl from "./wasm/tracerva_wasm_bg.wasm?url";
import type { TraceRequest, TraceResponse } from "./types";

self.onmessage = async (event: MessageEvent<TraceRequest>) => {
  try {
    await init({ module_or_path: wasmUrl });
    const started = performance.now();
    const { rgba, width, height, settings } = event.data;
    const result = JSON.parse(
      trace_rgba(rgba, width, height, JSON.stringify(settings)),
    );
    self.postMessage({
      ok: true,
      result,
      elapsed: performance.now() - started,
    } satisfies TraceResponse);
  } catch (error) {
    self.postMessage({
      ok: false,
      error: String(error),
    } satisfies TraceResponse);
  }
};
