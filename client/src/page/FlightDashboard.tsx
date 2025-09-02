"use client";

// Vite environment variables are already typed in node_modules/vite/types/importMeta.d.ts

import { useState, useEffect, useCallback, useMemo, useRef } from "react";
import {
  Chart,
  LineElement,
  PointElement,
  LineController,
  CategoryScale,
  LinearScale,
  Tooltip,
  Legend,
  Filler,
} from "chart.js";

// Usamos CategoryScale con timestamps formateados (sin adapter extra)
Chart.register(
  LineElement,
  PointElement,
  LineController,
  CategoryScale,
  LinearScale,
  Tooltip,
  Legend,
  Filler
);

// Config
const API_BASE: string =
  import.meta.env.VITE_API_BASE || "http://localhost:3000";

// Campos por defecto para gráficas principales
const DEFAULT_FIELDS = [
  "AngleRoll",
  "DesiredAngleRoll",
  "AnglePitch",
  "DesiredAnglePitch",
  "InputThrottle",
] as const;

// Tipos de API
export type FlightItem = {
  flight_id: string;
  last_ts: string;
};

export type SeriesPoint = {
  ts: string;
  values: Record<string, number>;
};

// WebSocket message type
interface WebSocketMessage {
  type: string;
  flight_id?: string;
  [key: string]: unknown;
}

// Chart data types
interface ChartDataset {
  label: string;
  data: (number | null)[];
  borderColor?: string;
  backgroundColor?: string;
  tension?: number;
  spanGaps?: boolean;
  pointRadius?: number;
  borderWidth?: number;
  fill?: boolean;
}

type ChartPack = {
  labels: string[];
  datasets: ChartDataset[];
};

// Tipos de API
interface FlightSummary {
  flight_id: string;
  start_ts: string;
  end_ts: string;
  duration_sec: number;
  max_roll?: number | null;
  max_pitch?: number | null;
  throttle_time_in_range_sec: number;
  throttle_time_out_range_sec: number;
}

export type FlightMetricsResponse = {
  flight_id: string;
  start_ts: string;
  end_ts: string;
  duration_sec: number;
  metrics: {
    rmse_roll?: number | null;
    rmse_pitch?: number | null;
    itae_roll?: number | null;
    itae_pitch?: number | null;
    mae_roll?: number | null;
    mae_pitch?: number | null;
    n_segments_used: number;
    duration_sec: number;
  };
  // Sugerencia de campos extra para graficar
  plot_fields: string[];
};

// Utils
const fmtTime = (iso: string) => new Date(iso).toLocaleString();
const fmtSec = (s: number) => {
  if (!isFinite(s)) return "-";
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = Math.floor(s % 60);
  const parts = [] as string[];
  if (h) parts.push(`${h}h`);
  if (m) parts.push(`${m}m`);
  parts.push(`${ss}s`);
  return parts.join(" ");
};
const fmtNum = (n?: number | null, digits = 3) =>
  n == null || !isFinite(n) ? "—" : Number(n).toFixed(digits);

// This code block should be inside the loadFlightData function

interface PointData {
  values?: Record<string, unknown>;
  ts?: string | number | Date;
  [key: string]: unknown;
}

function presentFieldsIn(points: PointData[], fields: string[]) {
  const present = new Set<string>();
  for (const p of points) {
    const obj: Record<string, unknown> =
      (p && typeof p === "object" && "values" in p ? p.values : p) ?? {};
    for (const [k, v] of Object.entries(obj)) {
      const n =
        typeof v === "number" ? v : typeof v === "string" ? Number(v) : NaN;
      if (Number.isFinite(n)) present.add(k);
    }
  }
  return fields.filter((f) => present.has(f));
}

function hashStr(s: string) {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h);
}

function colorFor(label: string) {
  const h = hashStr(label) % 360;
  return {
    stroke: `hsl(${h}, 70%, 50%)`,
    fill: `hsla(${h}, 70%, 50%, 0.10)`,
  };
}

// Construye datasets alineados con labels (uno por campo)
function buildDatasets(points: PointData[], fields: string[]): ChartPack {
  const labels = points.map((p) =>
    new Date(p.ts as string | number | Date).toLocaleTimeString()
  );

  const seriesByField: Record<string, (number | null)[]> = {};
  for (const f of fields) seriesByField[f] = [];

  for (const p of points) {
    const bag: Record<string, unknown> =
      p && typeof p === "object" && "values" in p
        ? (p.values as Record<string, unknown>)
        : p?.values ?? p ?? {};

    for (const f of fields) {
      const raw = bag?.[f];
      const n =
        typeof raw === "number"
          ? raw
          : typeof raw === "string"
          ? Number(raw)
          : NaN;
      seriesByField[f].push(Number.isFinite(n) ? n : null);
    }
  }

  const datasets = fields.map((f) => {
    const { stroke, fill } = colorFor(f);
    return {
      label: f,
      data: seriesByField[f],
      borderColor: stroke,
      backgroundColor: fill,
      tension: 0.2,
      spanGaps: true,
      pointRadius: 0,
      borderWidth: 2,
      fill: false,
    };
  });

  return { labels, datasets };
}

// Pequeño wrapper de Chart.js para no re-renderizar innecesariamente
function LineChart({
  title,
  labels,
  datasets,
  height = 220,
}: {
  title: string;
  labels: string[];
  datasets: ChartDataset[];
  height?: number;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const chartRef = useRef<Chart | null>(null);

  useEffect(() => {
    if (!canvasRef.current) return;
    const ctx = canvasRef.current.getContext("2d");
    if (!ctx) return;

    if (chartRef.current) chartRef.current.destroy();
    chartRef.current = new Chart(ctx, {
      type: "line",
      data: { labels, datasets },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        animation: false,
        plugins: {
          legend: { display: true, position: "bottom" },
          title: { display: !!title, text: title },
          tooltip: { intersect: false, mode: "index" as const },
        },
        elements: { point: { radius: 0 } },
        scales: {
          x: { ticks: { autoSkip: true, maxTicksLimit: 9 } },
          y: { beginAtZero: false },
        },
      },
    });

    return () => {
      chartRef.current?.destroy();
      chartRef.current = null;
    };
  }, [labels, datasets, title]);

  return (
    <div className="w-full" style={{ height }}>
      <canvas ref={canvasRef} />
    </div>
  );
}

// API helpers
async function apiJson<T>(url: string): Promise<T> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
  return (await r.json()) as T;
}

async function fetchFlights(limit = 20): Promise<FlightItem[]> {
  const items = await apiJson<FlightItem[]>(
    `${API_BASE}/api/flights?limit=${limit}`
  );
  return items.sort((a, b) => +new Date(b.last_ts) - +new Date(a.last_ts));
}

async function fetchSeries(fid: string, fields: string[], limit = 50000) {
  const qs = new URLSearchParams({
    fields: fields.join(","),
    limit: String(limit),
  });
  return apiJson<SeriesPoint[]>(
    `${API_BASE}/api/flights/${encodeURIComponent(fid)}/series?${qs}`
  );
}

async function fetchSummary(
  fid: string,
  params?: { throttle_min?: number; throttle_max?: number }
) {
  const qs = new URLSearchParams();
  if (params?.throttle_min != null)
    qs.set("throttle_min", String(params.throttle_min));
  if (params?.throttle_max != null)
    qs.set("throttle_max", String(params.throttle_max));
  const url = `${API_BASE}/api/flights/${encodeURIComponent(fid)}/summary${
    qs.toString() ? `?${qs}` : ""
  }`;
  return apiJson<FlightSummary>(url);
}

async function fetchMetrics(fid: string) {
  return apiJson<FlightMetricsResponse>(
    `${API_BASE}/api/flights/${encodeURIComponent(fid)}/metrics`
  );
}

// Página principal
export default function FlightDashboard() {
  const [flights, setFlights] = useState<FlightItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const [series, setSeries] = useState<SeriesPoint[] | null>(null);
  const [extraSeries, setExtraSeries] = useState<SeriesPoint[] | null>(null);
  const [summary, setSummary] = useState<FlightSummary | null>(null);
  const [metrics, setMetrics] = useState<FlightMetricsResponse | null>(null);

  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | string | null>(null);

  const loadFlights = useCallback(async () => {
    try {
      const list = await fetchFlights(20);
      setFlights(list);
      if (!selectedId && list.length) setSelectedId(list[0].flight_id);
    } catch (error: unknown) {
      console.error("Error fetching flight data:", error);
      setError(error instanceof Error ? error : String(error));
    }
  }, [selectedId]);

  // === Campos disponibles (para filtrar extras) ===
  const [availableFields, setAvailableFields] = useState<string[] | null>(null);

  const fetchAvailableFields = useCallback(async () => {
    try {
      const r = await fetch(`${API_BASE}/api/telemetry/fields`);
      if (!r.ok) throw new Error("fields fetch failed");
      const data = (await r.json()) as {
        fields: string[];
        last_updated: string;
      };
      setAvailableFields(data.fields);
    } catch (e) {
      console.debug("fields fetch failed", e);
    }
  }, []);

  useEffect(() => {
    fetchAvailableFields();
    const id = setInterval(fetchAvailableFields, 10000);
    return () => clearInterval(id);
  }, [fetchAvailableFields]);

  const loadFlightData = useCallback(
    async (fid: string) => {
      if (!fid) return;
      setLoading(true);
      setError(null);
      console.log('Loading flight data for ID:', fid);
      try {
        // 1) Datos base (incluye ángulos deseados)
        const mainFields = [...DEFAULT_FIELDS];
        // 2) Métricas + campos extra
        const [srs, sum, met] = await Promise.all([
          fetchSeries(fid, mainFields as string[]),
          fetchSummary(fid),
          fetchMetrics(fid),
        ]);

        setSeries(srs);
        setSummary(sum);
        setMetrics(met);

        // Always include these fields in the request
        const defaultExtras = [
          "AccX",
          "AccY",
          "AccZ",
          "DesiredAnglePitch",
          "DesiredAngleRoll",
          "DesiredRateYaw",
          "g1",
          "g2",
          "k1",
          "k2",
          "m1",
          "m2",
          "tau_x",
          "tau_y",
          "tau_z",
        ];

        const requestedExtras = Array.isArray(met?.plot_fields) && met.plot_fields.length > 0
          ? [...new Set([...met.plot_fields, ...defaultExtras])] // Merge and dedupe
          : defaultExtras;

        // Si el backend expuso catálogo, filtramos para evitar 500 por columnas inexistentes
        const extrasToAsk = availableFields
          ? requestedExtras.filter((f) => availableFields.includes(f))
          : requestedExtras;

        console.log('Requesting extra fields:', extrasToAsk);
        const extra = extrasToAsk.length
          ? await fetchSeries(fid, extrasToAsk)
          : [];
        console.log('Received extra series data:', extra);
        setExtraSeries(extra);
      } catch (error: unknown) {
        console.error("Error fetching flight data:", error);
        setError(error instanceof Error ? error : String(error));
        setSeries(null);
        setSummary(null);
        setMetrics(null);
        setExtraSeries(null);
      } finally {
        setLoading(false);
      }
    },
    [availableFields]
  );

  // Inicial: lista + auto-selección del más reciente
  useEffect(() => {
    loadFlights();
  }, [loadFlights]);

  // Cuando cambia el vuelo seleccionado, carga datos
  useEffect(() => {
    if (selectedId) loadFlightData(selectedId);
  }, [selectedId, loadFlightData]);

  // Poll suave para nuevos vuelos finalizados (cada 5s)
  useEffect(() => {
    let last = flights[0]?.flight_id || null;
    const id = setInterval(async () => {
      try {
        const list = await fetchFlights(1);
        const newest = list[0]?.flight_id;
        if (newest && newest !== last) {
          last = newest;
          setFlights((prev) => {
            const merged = [
              ...list,
              ...prev.filter((p) => p.flight_id !== newest),
            ];
            return merged.sort(
              (a, b) => +new Date(b.last_ts) - +new Date(a.last_ts)
            );
          });
          setSelectedId(newest);
        }
      } catch (error) {
        console.debug("Error polling for new flights:", error);
      }
    }, 5000);
    return () => clearInterval(id);
    // sin deps → un solo interval
  }, [flights]);

  // WebSocket: escuchar recording_stopped para saltar al último vuelo
  useEffect(() => {
    const url =
      import.meta.env.VITE_WS_URL ||
      `${location.protocol === "https:" ? "wss" : "ws"}://${
        location.hostname
      }:9001/`;

    console.log("Attempting to connect to WebSocket at:", url);

    let ws: WebSocket | null = null;
    let closedByEffect = false;

    const open = () => {
      ws = new WebSocket(url);

      ws.onopen = () => {
        console.log("WebSocket connection established");
      };

      ws.onmessage = (ev) => {
        try {
          const msg = JSON.parse(ev.data) as WebSocketMessage;
          if (msg?.type === "recording_stopped" && msg.flight_id) {
            console.log("Recording stopped, flight ID:", msg.flight_id);
            setSelectedId(msg.flight_id);
            // refresca lista al terminar un vuelo
            loadFlights();
          }
        } catch (e) {
          console.warn("WS message parse error:", e);
        }
      };

      ws.onerror = (e) => {
        console.warn("WebSocket error:", e);
      };

      ws.onclose = (e) => {
        if (!closedByEffect) {
          console.warn("WebSocket closed:", e.code, e.reason || "");
          // backoff simple
          setTimeout(open, 1000);
        }
      };
    };

    open();

    return () => {
      closedByEffect = true;
      try {
        ws?.close();
      } catch (e) {
        console.debug("ws close error", e);
      }
      ws = null;
      console.log("Cleaning up WebSocket connection");
    };
  }, [loadFlights]);

  // ====== datasets ======
  interface ChartsData {
    roll: ChartPack;
    pitch: ChartPack;
    thr: ChartPack;
    extras: {
      acc: ChartPack;
      yaw: ChartPack;
      g: ChartPack;
      k: ChartPack;
      m: ChartPack;
      tau: ChartPack;
    } | null;
  }

  const charts = useMemo<ChartsData | null>(() => {
    console.log('Regenerating charts with series:', series, 'and extraSeries:', extraSeries);
    
    // Debug: Log the structure of the first data point
    if (extraSeries && extraSeries.length > 0) {
      console.log('First extraSeries point structure:', JSON.stringify(extraSeries[0], null, 2));
      if (extraSeries[0].values) {
        console.log('Available fields in first point:', Object.keys(extraSeries[0].values));
      }
    }
    
    if (!series || !series.length) {
      console.log('No series data available');
      return null;
    }

    // Helper function to create chart data with proper typing
    const createChartData = (
      data: SeriesPoint[],
      fields: string[]
    ): ChartPack => {
      const result = buildDatasets(data, fields);
      return {
        labels: result.labels,
        datasets: result.datasets.map((ds) => {
          const c = colorFor(ds.label);
          return {
            ...ds,
            borderColor: c.stroke,
            backgroundColor: c.fill,
            tension: 0.2,
            spanGaps: true,
            pointRadius: 0,
            borderWidth: 2,
            fill: false,
          };
        }),
      };
    };

    // Attitude comparativa (ángulo vs deseado)
    const roll = createChartData(
      series,
      presentFieldsIn(series, ["AngleRoll", "DesiredAngleRoll"])
    );

    const pitch = createChartData(
      series,
      presentFieldsIn(series, ["AnglePitch", "DesiredAnglePitch"])
    );

    const thr = createChartData(
      series,
      presentFieldsIn(series, ["InputThrottle"])
    );

    // Extras (si existen)
    const extras =
      extraSeries && extraSeries.length
        ? {
            acc: createChartData(
              extraSeries,
              presentFieldsIn(extraSeries, ["AccX", "AccY", "AccZ"])
            ),
            yaw: createChartData(
              extraSeries,
              presentFieldsIn(extraSeries, ["DesiredRateYaw"])
            ),
            g: createChartData(
              extraSeries,
              presentFieldsIn(extraSeries, ["g1", "g2"])
            ),
            k: createChartData(
              extraSeries,
              presentFieldsIn(extraSeries, ["k1", "k2"])
            ),
            m: createChartData(
              extraSeries,
              presentFieldsIn(extraSeries, ["m1", "m2"])
            ),
            tau: (() => {
              const tauFields = ["tau_x", "tau_y", "tau_z"].filter(field => {
                const exists = extraSeries.some(point => {
                  const hasField = point.values && point.values[field] !== undefined && point.values[field] !== null;
                  if (hasField) {
                    console.log(`Found field ${field} in data`);
                  }
                  return hasField;
                });
                if (!exists) {
                  console.log(`Field ${field} not found in any data point`);
                }
                return exists;
              });
              console.log('Tau fields to plot:', tauFields);
              return createChartData(extraSeries, tauFields);
            })(),
          }
        : null;

    return { roll, pitch, thr, extras };
  }, [series, extraSeries]);

  return (
    <div className="p-6 text-white max-w-6xl mx-auto space-y-6">
      <div className="flex flex-col md:flex-row md:items-center md:justify-between gap-3">
        <h1 className="text-3xl font-bold">📈 Métricas de Vuelo</h1>
        <div className="flex items-center gap-2">
          <select
            value={selectedId ?? ""}
            onChange={(e) => setSelectedId(e.target.value || null)}
            className="bg-gray-800 border border-gray-700 rounded px-3 py-2 text-sm"
          >
            {!selectedId && <option value="">Selecciona vuelo…</option>}
            {flights.map((f) => (
              <option key={f.flight_id} value={f.flight_id}>
                {f.flight_id.slice(0, 8)} · {fmtTime(f.last_ts)}
              </option>
            ))}
          </select>
          <button
            onClick={() =>
              selectedId ? loadFlightData(selectedId) : loadFlights()
            }
            className="px-3 py-2 rounded bg-gray-700 hover:bg-gray-600 text-sm"
          >
            Refrescar
          </button>
        </div>
      </div>

      {error && (
        <div className="p-4 bg-red-100 text-red-800 rounded-lg">
          Error:{" "}
          {typeof error === "string"
            ? error
            : error?.message || "Unknown error"}
        </div>
      )}

      {/* Bloque grande arriba con info del vuelo */}
      {summary && (
        <div className="bg-gray-800 p-6 rounded-2xl border border-gray-700 mb-6">
          <div className="text-xs text-gray-400 mb-1">Vuelo</div>
          <div className="text-xl font-mono break-all">{selectedId || "—"}</div>
          <div className="text-sm text-gray-400 mt-2">
            {fmtTime(summary.start_ts)} → {fmtTime(summary.end_ts)}
          </div>
        </div>
      )}

      {/* KPIs principales abajo */}
      <div className="grid grid-cols-1 md:grid-cols-5 gap-4">
        <StatCard
          label="Duración"
          value={summary ? fmtSec(summary.duration_sec) : "—"}
        />
        <StatCard
          label="RMSE Roll"
          value={fmtNum(metrics?.metrics.rmse_roll)}
        />
        <StatCard
          label="RMSE Pitch"
          value={fmtNum(metrics?.metrics.rmse_pitch)}
        />
        <StatCard
          label="ITAE Roll"
          value={fmtNum(metrics?.metrics.itae_roll)}
        />
        <StatCard
          label="ITAE Pitch"
          value={fmtNum(metrics?.metrics.itae_pitch)}
        />
      </div>

      {/* Gráficas: Comparativas de ángulos y Throttle */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        <div className="bg-gray-800 p-4 rounded-2xl border border-gray-700">
          <div className="text-sm text-gray-300 mb-2">Roll vs Desired</div>
          {charts?.roll?.datasets?.length ? (
            <LineChart
              title="Roll"
              labels={charts.roll.labels}
              datasets={charts.roll.datasets}
              height={260}
            />
          ) : (
            <EmptyState loading={loading} />
          )}
        </div>
        <div className="bg-gray-800 p-4 rounded-2xl border border-gray-700">
          <div className="text-sm text-gray-300 mb-2">Pitch vs Desired</div>
          {charts?.pitch?.datasets?.length ? (
            <LineChart
              title="Pitch"
              labels={charts.pitch.labels}
              datasets={charts.pitch.datasets}
              height={260}
            />
          ) : (
            <EmptyState loading={loading} />
          )}
        </div>
        <div className="bg-gray-800 p-4 rounded-2xl border border-gray-700 md:col-span-2">
          <div className="text-sm text-gray-300 mb-2">InputThrottle</div>
          {charts?.thr?.datasets?.length ? (
            <LineChart
              title="Throttle"
              labels={charts.thr.labels}
              datasets={charts.thr.datasets}
              height={220}
            />
          ) : (
            <EmptyState loading={loading} />
          )}
        </div>
      </div>

      {/* Extras */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        <PanelChart
          title="Acelerómetros (g)"
          pack={charts?.extras?.acc}
          loading={loading}
        />
        <PanelChart
          title="DesiredRateYaw"
          pack={charts?.extras?.yaw}
          loading={loading}
        />
        <PanelChart
          title="g1 / g2"
          pack={charts?.extras?.g}
          loading={loading}
        />
        <PanelChart
          title="k1 / k2"
          pack={charts?.extras?.k}
          loading={loading}
        />
        <PanelChart
          title="m1 / m2"
          pack={charts?.extras?.m}
          loading={loading}
        />
        <PanelChart
          title="τx / τy / τz"
          pack={charts?.extras?.tau}
          loading={loading}
        />
      </div>
    </div>
  );
}

interface PanelChartProps {
  title: string;
  pack: ChartPack | null | undefined;
  loading: boolean;
}

function PanelChart({ title, pack, loading }: PanelChartProps) {
  return (
    <div className="bg-gray-800 p-4 rounded-2xl border border-gray-700">
      <div className="text-sm text-gray-300 mb-2">{title}</div>
      {pack && pack.datasets?.length ? (
        <LineChart
          title={title}
          labels={pack.labels}
          datasets={pack.datasets}
          height={220}
        />
      ) : (
        <EmptyState loading={loading} />
      )}
    </div>
  );
}

function StatCard({
  label,
  value,
  sub,
}: {
  label: string;
  value: string;
  sub?: string;
}) {
  return (
    <div className="bg-gray-800 p-4 rounded-2xl border border-gray-700">
      <div className="text-xs text-gray-400">{label}</div>
      <div className="text-2xl font-semibold">{value}</div>
      {sub ? <div className="text-xs text-gray-400 mt-1">{sub}</div> : null}
    </div>
  );
}

function EmptyState({ loading }: { loading: boolean }) {
  return (
    <div className="h-[220px] flex items-center justify-center text-gray-400 text-sm">
      {loading ? "Cargando…" : "Sin datos"}
    </div>
  );
}
