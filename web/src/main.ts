import "./style.css";
import type { Settings, TraceRequest, TraceResponse } from "./types";

function el<T extends HTMLElement>(id: string): T {
  return document.getElementById(id) as T;
}
const source = el<HTMLImageElement>("source");
const result = el<HTMLImageElement>("result");
const fileInput = el<HTMLInputElement>("file");
const convert = el<HTMLButtonElement>("convert");
const cancel = el<HTMLButtonElement>("cancel");
const download = el<HTMLAnchorElement>("download");
const controls = el<HTMLFieldSetElement>("controls");
const zoom = el<HTMLSelectElement>("zoom");
const sourceView = el<HTMLDivElement>("source-view");
const resultView = el<HTMLDivElement>("result-view");
let pixels: ImageData | null = null;
let sourceUrl = "";
let resultUrl = "";
let filename = "tracerva";
let worker: Worker | null = null;
let loading = false;
let loadVersion = 0;

function status(text: string, error = false) {
  el("status").textContent = text;
  el("status").classList.toggle("error", error);
}
function clearResult() {
  if (resultUrl) URL.revokeObjectURL(resultUrl);
  resultUrl = "";
  result.removeAttribute("src");
  result.hidden = true;
  el("result-empty").hidden = false;
  download.removeAttribute("href");
  download.classList.add("disabled");
  download.setAttribute("aria-disabled", "true");
}
function busy(value: boolean) {
  controls.disabled = value;
  fileInput.disabled = value;
  el<HTMLButtonElement>("sample").disabled = value;
  convert.disabled = value || !pixels || loading;
  convert.textContent = value ? "変換しています…" : "SVGに変換";
  cancel.hidden = !value;
}
function stopWorker() {
  worker?.terminate();
  worker = null;
  busy(false);
}

async function loadImage(file: Blob, name: string) {
  if (worker) return;
  const version = ++loadVersion;
  loading = true;
  convert.disabled = true;
  status("画像を読み込んでいます…");
  let bitmap: ImageBitmap | undefined;
  try {
    if (!["image/png", "image/jpeg", "image/webp"].includes(file.type))
      throw new Error("PNG・JPEG・WebPの画像を選んでください。");
    if (file.size > 30 * 1024 * 1024)
      throw new Error("30MB以下の画像を選んでください。");
    bitmap = await createImageBitmap(file, { imageOrientation: "from-image" });
    if (version !== loadVersion) return;
    const { width, height } = bitmap;
    if (!width || !height || width * height > 16_000_000)
      throw new Error(
        "画像は最大1,600万画素です。縮小してから読み込んでください。",
      );
    const canvas = document.createElement("canvas");
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    if (!context) throw new Error("このブラウザでは画像を読み込めません。");
    context.drawImage(bitmap, 0, 0);
    const nextPixels = context.getImageData(0, 0, width, height);
    // Use the same decoded raster for both display and tracing (including EXIF orientation).
    const normalized = await new Promise<Blob>((resolve, reject) =>
      canvas.toBlob(
        (blob) =>
          blob
            ? resolve(blob)
            : reject(new Error("画像の準備に失敗しました。")),
        "image/png",
      ),
    );
    if (version !== loadVersion) return;
    if (sourceUrl) URL.revokeObjectURL(sourceUrl);
    pixels = nextPixels;
    filename = name.replace(/\.[^.]+$/, "") || "tracerva";
    sourceUrl = URL.createObjectURL(normalized);
    source.src = sourceUrl;
    source.hidden = false;
    el("source-empty").hidden = true;
    el("filename").textContent = `${name} · ${width} × ${height}`;
    clearResult();
    applyZoom();
    status("設定を選んで「SVGに変換」を押してください。");
  } catch (error) {
    if (version === loadVersion)
      status(error instanceof Error ? error.message : String(error), true);
  } finally {
    bitmap?.close();
    if (version === loadVersion) {
      loading = false;
      busy(false);
    }
  }
}

fileInput.addEventListener("change", () => {
  const file = fileInput.files?.[0];
  if (file) void loadImage(file, file.name);
  fileInput.value = "";
});
const dropzone = el("dropzone");
for (const event of ["dragenter", "dragover"])
  dropzone.addEventListener(event, (e) => {
    e.preventDefault();
    dropzone.classList.add("dragging");
  });
for (const event of ["dragleave", "drop"])
  dropzone.addEventListener(event, (e) => {
    e.preventDefault();
    dropzone.classList.remove("dragging");
  });
dropzone.addEventListener("drop", (e) => {
  const file = (e as DragEvent).dataTransfer?.files[0];
  if (file && !worker) void loadImage(file, file.name);
});
// Avoid browser navigation if an image is dropped outside the upload area.
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", (e) => e.preventDefault());

el("sample").addEventListener("click", () => {
  el<HTMLSelectElement>("color-mode").value = "palette";
  el<HTMLInputElement>("palette").value = "#ffffff,#1b6658";
  updateColorMode();
  const canvas = document.createElement("canvas");
  canvas.width = 960;
  canvas.height = 640;
  const ctx = canvas.getContext("2d")!;
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, 960, 640);
  ctx.fillStyle = "#1b6658";
  ctx.beginPath();
  ctx.arc(480, 280, 180, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = "#ffffff";
  ctx.beginPath();
  ctx.arc(480, 280, 125, 0, Math.PI * 2);
  ctx.fill();
  ctx.fillStyle = "#1b6658";
  ctx.beginPath();
  ctx.moveTo(480, 190);
  ctx.lineTo(410, 330);
  ctx.lineTo(550, 330);
  ctx.closePath();
  ctx.fill();
  for (const x of [165, 795]) {
    ctx.fillStyle = "#1b6658";
    ctx.beginPath();
    ctx.arc(x, 280, 22, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.strokeStyle = "#1b6658";
  ctx.lineWidth = 12;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(220, 515);
  ctx.bezierCurveTo(340, 435, 620, 595, 740, 515);
  ctx.stroke();
  canvas.toBlob((blob) => {
    if (blob) void loadImage(blob, "tracerva-sample.png");
  }, "image/png");
});

el<HTMLInputElement>("smooth").addEventListener("input", (e) => {
  el("smooth-value").textContent = `${(e.target as HTMLInputElement).value} px`;
});
controls.addEventListener("input", () => {
  if (resultUrl) {
    clearResult();
    status("設定が変わりました。もう一度変換してください。");
  }
});

function updateColorMode() {
  const mode = el<HTMLSelectElement>("color-mode").value;
  el("palette-label").hidden = mode !== "palette";
  el<HTMLInputElement>("colors").disabled = mode !== "auto";
  el<HTMLInputElement>("merge").disabled = mode !== "auto";
}
el("color-mode").addEventListener("change", updateColorMode);

el<HTMLFormElement>("settings").addEventListener("submit", (e) => {
  e.preventDefault();
  if (!pixels || worker || loading) return;
  clearResult();
  const hex = el<HTMLInputElement>("background").value;
  const mode = el<HTMLSelectElement>("color-mode").value;
  let palette: [number, number, number][] = [];
  if (mode === "mono")
    palette = [
      [0, 0, 0],
      [255, 255, 255],
    ];
  if (mode === "palette") {
    const entries = el<HTMLInputElement>("palette")
      .value.split(",")
      .map((s) => s.trim().replace(/^#/, ""));
    if (
      !entries.length ||
      entries.length > 256 ||
      entries.some((s) => !/^[0-9a-f]{6}$/i.test(s))
    ) {
      status(
        "指定色は #ffffff,#123953 のように6桁の16進色をカンマで区切ってください（最大256色）。",
        true,
      );
      return;
    }
    palette = entries.map(
      (s) =>
        [0, 2, 4].map((i) => parseInt(s.slice(i, i + 2), 16)) as [
          number,
          number,
          number,
        ],
    );
  }
  const settings: Settings = {
    colors: Number(el<HTMLInputElement>("colors").value),
    smooth: Number(el<HTMLInputElement>("smooth").value),
    merge_distance:
      mode === "auto" ? Number(el<HTMLInputElement>("merge").value) : 0,
    palette,
    geometry: el<HTMLInputElement>("geometry").checked,
    background: el<HTMLInputElement>("remove-bg").checked
      ? ([1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as [
          number,
          number,
          number,
        ])
      : null,
    outline: el<HTMLInputElement>("outline").checked,
  };
  busy(true);
  status("輪郭をトレースしています。大きな画像は時間がかかる場合があります。");
  try {
    worker = new Worker(new URL("./worker.ts", import.meta.url), {
      type: "module",
    });
    worker.onmessage = (event: MessageEvent<TraceResponse>) => {
      const response = event.data;
      stopWorker();
      if (!response.ok) {
        status(`変換できませんでした: ${response.error}`, true);
        return;
      }
      const { svg, paths, circles, polygons } = response.result;
      const blob = new Blob([svg], { type: "image/svg+xml" });
      resultUrl = URL.createObjectURL(blob);
      result.src = resultUrl;
      result.hidden = false;
      el("result-empty").hidden = true;
      download.href = resultUrl;
      download.download = `${filename}.svg`;
      download.classList.remove("disabled");
      download.setAttribute("aria-disabled", "false");
      applyZoom();
      status(
        `${paths} パス · ${(blob.size / 1024).toFixed(1)} KB · ${(response.elapsed / 1000).toFixed(2)} 秒 / 幾何補正: 円 ${circles}・多角形 ${polygons}`,
      );
    };
    worker.onerror = () => {
      stopWorker();
      status(
        "変換処理を開始できませんでした。再試行するか、画像を小さくしてください。",
        true,
      );
    };
    const rgba = new Uint8Array(pixels.data);
    worker.postMessage(
      {
        rgba,
        width: pixels.width,
        height: pixels.height,
        settings,
      } satisfies TraceRequest,
      [rgba.buffer],
    );
  } catch (error) {
    stopWorker();
    status(`変換を開始できませんでした: ${String(error)}`, true);
  }
});
cancel.addEventListener("click", () => {
  stopWorker();
  status("変換を中止しました。設定を変えて再試行できます。");
});
download.addEventListener("click", (e) => {
  if (!resultUrl) e.preventDefault();
});

function applyZoom() {
  if (!pixels) return;
  const scale =
    zoom.value === "fit"
      ? Math.min(
          1,
          (sourceView.clientWidth - 32) / pixels.width,
          (sourceView.clientHeight - 32) / pixels.height,
        )
      : Number(zoom.value);
  for (const img of [source, result]) {
    img.style.width = `${Math.max(1, pixels.width * scale)}px`;
    img.style.height = `${Math.max(1, pixels.height * scale)}px`;
  }
}
zoom.addEventListener("change", applyZoom);
new ResizeObserver(applyZoom).observe(sourceView);
for (const [from, to] of [
  [sourceView, resultView],
  [resultView, sourceView],
]) {
  from.addEventListener(
    "scroll",
    () => {
      if (Math.abs(to.scrollLeft - from.scrollLeft) > 1)
        to.scrollLeft = from.scrollLeft;
      if (Math.abs(to.scrollTop - from.scrollTop) > 1)
        to.scrollTop = from.scrollTop;
    },
    { passive: true },
  );
}
