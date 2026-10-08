export interface Settings {
  colors: number;
  smooth: number;
  geometry: boolean;
  merge_distance: number;
  background: [number, number, number] | null;
  outline: boolean;
  palette: [number, number, number][];
}
export interface TraceRequest {
  rgba: Uint8Array;
  width: number;
  height: number;
  settings: Settings;
}
export interface TraceResult {
  svg: string;
  paths: number;
  circles: number;
  polygons: number;
  colors: number;
}
export type TraceResponse =
  | { ok: true; result: TraceResult; elapsed: number }
  | { ok: false; error: string };
