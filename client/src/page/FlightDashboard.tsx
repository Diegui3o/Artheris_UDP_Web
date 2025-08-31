"use client";
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
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
  TimeScale,
} from "chart.js";

// (Opcional) si usas chartjs-adapter-date-fns o moment, podrías registrar TimeScale
// pero para mantenerlo simple usaremos CategoryScale con strings formateados.
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

// =====================
// Config
// =====================
const API_BASE: string =
  (import.meta as any)?.env?.VITE_API_BASE || "http://localhost:3000";
const WS_URL: string =
  (import.meta as any)?.env?.VITE_WS_URL ||
  API_BASE.replace(/^http/, "ws") + "/ws";

// Campos por defecto a graficar
const DEFAULT_FIELDS = ["AngleRoll", "AnglePitch", "InputThrottle"] as const;

// =====================
// Tipos de API
// =====================
export type FlightItem = { flight_id: string; last_ts: string };
export type SeriesPoint = { ts: string; values: Record<string, number> };
export type FlightSummary = {
  flight_id: string;
  start_ts: string;
  end_ts: string;
  duration_sec: number;
  max_roll?: number | null;
  max_pitch?: number | null;
  throttle_time_in_range_sec: number;
  throttle_time_out_range_sec: number;
};

// =====================
// Utils
// =====================
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

// Construye datasets alineados con labels (uno por campo)
function buildDatasets(points: SeriesPoint[], fields: string[]) {
  const labels = points.map((p) => new Date(p.ts).toLocaleTimeString());
  const seriesByField: Record<string, (number | null)[]> = {};
  for (const f of fields) seriesByField[f] = [];

  for (const p of points) {
    for (const f of fields) {
      const v = p.values?.[f];
      seriesByField[f].push(typeof v === "number" && isFinite(v) ? v : null);
    }
  }

  const datasets = fields.map((f) => ({
    label: f,
    data: seriesByField[f],
    tension: 0.2,
    spanGaps: true,
    pointRadius: 0,
    borderWidth: 2,
    fill: false,
  }));
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
  datasets: any[];
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

// =====================
// API helpers
// =====================
async function apiJson<T>(url: string): Promise<T> {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
  return (await r.json()) as T;
}

async function fetchFlights(limit = 20): Promise<FlightItem[]> {
  const items = await apiJson<FlightItem[]>(
    `${API_BASE}/api/flights?limit=${limit}`
  );
  // Ordena por last_ts desc por si acaso
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

// =====================
// Página principal
// =====================
export default function FlightDashboard() {
  const [flights, setFlights] = useState<FlightItem[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [series, setSeries] = useState<SeriesPoint[] | null>(null);
  const [summary, setSummary] = useState<FlightSummary | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadFlights = useCallback(async () => {
    try {
      const list = await fetchFlights(20);
      setFlights(list);
      if (!selectedId && list.length) setSelectedId(list[0].flight_id);
    } catch (e: any) {
      console.error(e);
      setError(e?.message || String(e));
    }
  }, [selectedId]);

  const loadFlightData = useCallback(async (fid: string) => {
    setLoading(true);
    setError(null);
    try {
      const [srs, sum] = await Promise.all([
        fetchSeries(fid, [...DEFAULT_FIELDS]),
        fetchSummary(fid),
      ]);
      setSeries(srs);
      setSummary(sum);
    } catch (e: any) {
      console.error(e);
      setError(e?.message || String(e));
      setSeries(null);
      setSummary(null);
    } finally {
      setLoading(false);
    }
  }, []);

  // Inicial: lista + auto-selección del más reciente
  useEffect(() => {
    loadFlights();
  }, [loadFlights]);

  // Cuando cambia el vuelo seleccionado, carga datos
  useEffect(() => {
    if (selectedId) loadFlightData(selectedId);
  }, [selectedId, loadFlightData]);

  // Poll suave para detectar nuevos vuelos finalizados (cada 5s)
  useEffect(() => {
    const id = setInterval(async () => {
      try {
        const list = await fetchFlights(1);
        const newest = list[0]?.flight_id;
        if (newest && newest !== flights[0]?.flight_id) {
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
      } catch (e) {
        // ignora errores de poll
      }
    }, 5000);
    return () => clearInterval(id);
  }, [flights]);

  // WebSocket: si existe /ws, escucha recording_stopped para saltar al último vuelo
  useEffect(() => {
    let ws: WebSocket | null = null;
    try {
      ws = new WebSocket(WS_URL);
      ws.onmessage = (ev) => {
        try {
          const msg = JSON.parse(ev.data);
          if (msg?.type === "recording_stopped" && msg.flight_id) {
            // nuevo vuelo terminado → recarga lista y selecciona ese
            setSelectedId(msg.flight_id as string);
            loadFlights();
          }
        } catch {}
      };
    } catch (e) {
      // si falla el WS, el poll ya cubre el caso
    }
    return () => {
      try {
        ws?.close();
      } catch {}
    };
  }, [loadFlights]);

  const labelsAndDatasets = useMemo(() => {
    if (!series || !series.length) return null;
    return {
      attitude: buildDatasets(series, ["AngleRoll", "AnglePitch"]),
      throttle: buildDatasets(series, ["InputThrottle"]),
    };
  }, [series]);

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
        <div className="bg-red-900/30 border border-red-700 text-red-200 p-3 rounded">
          Error: {error}
        </div>
      )}

      {/* Resumen */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-4">
        <StatCard
          label="Vuelo"
          value={selectedId ? selectedId.slice(0, 8) : "—"}
          sub={
            summary
              ? `${fmtTime(summary.start_ts)} → ${fmtTime(summary.end_ts)}`
              : ""
          }
        />
        <StatCard
          label="Duración"
          value={summary ? fmtSec(summary.duration_sec) : "—"}
        />
        <StatCard
          label="Max |Roll|"
          value={
            summary?.max_roll != null ? `${summary.max_roll?.toFixed(2)}°` : "—"
          }
        />
        <StatCard
          label="Max |Pitch|"
          value={
            summary?.max_pitch != null
              ? `${summary.max_pitch?.toFixed(2)}°`
              : "—"
          }
        />
      </div>

      {/* Gráficas */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        <div className="bg-gray-800 p-4 rounded-2xl border border-gray-700">
          <div className="text-sm text-gray-300 mb-2">Ángulos (°)</div>
          {labelsAndDatasets?.attitude ? (
            <LineChart
              title="Attitude: Roll & Pitch"
              labels={labelsAndDatasets.attitude.labels}
              datasets={labelsAndDatasets.attitude.datasets}
              height={260}
            />
          ) : (
            <EmptyState loading={loading} />
          )}
        </div>
        <div className="bg-gray-800 p-4 rounded-2xl border border-gray-700">
          <div className="text-sm text-gray-300 mb-2">Throttle</div>
          {labelsAndDatasets?.throttle ? (
            <LineChart
              title="InputThrottle"
              labels={labelsAndDatasets.throttle.labels}
              datasets={labelsAndDatasets.throttle.datasets}
              height={260}
            />
          ) : (
            <EmptyState loading={loading} />
          )}
        </div>
      </div>

      {/* Breakdown throttle in/out of range */}
      <div className="bg-gray-800 p-4 rounded-2xl border border-gray-700">
        <div className="text-sm text-gray-300 mb-2">Tiempo de throttle</div>
        {summary ? (
          <div className="grid grid-cols-2 gap-4">
            <BarRow
              label="En rango"
              valueSec={summary.throttle_time_in_range_sec}
            />
            <BarRow
              label="Fuera de rango"
              valueSec={summary.throttle_time_out_range_sec}
            />
          </div>
        ) : (
          <EmptyState loading={loading} />
        )}
      </div>
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

function BarRow({ label, valueSec }: { label: string; valueSec: number }) {
  const pct = useMemo(() => {
    // normaliza contra sumatoria (evitar divide-by-zero). El contenedor no conoce el total; se usa un ancho fijo relativo.
    // Muestra una barra proporcional simple tomando log para hacerla visual si hay grandes asimetrías.
    const v = Math.max(0, valueSec);
    return Math.min(100, (Math.log10(1 + v) / Math.log10(1 + v + 1)) * 100);
  }, [valueSec]);
  return (
    <div>
      <div className="flex items-center justify-between text-sm mb-1">
        <span className="text-gray-300">{label}</span>
        <span className="text-gray-400">{fmtSec(valueSec)}</span>
      </div>
      <div className="w-full h-2 bg-gray-700 rounded-full overflow-hidden">
        <div
          className="h-2 rounded-full"
          style={{ width: `${pct}%`, background: "currentColor" }}
        />
      </div>
    </div>
  );
}
