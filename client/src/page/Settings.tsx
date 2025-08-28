"use client";
import { registry } from "../types/telemetryRegistry";
import { useEffect, useMemo, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import {
  Chart,
  LineElement,
  PointElement,
  LineController,
  CategoryScale,
  LinearScale,
  Tooltip,
  Legend,
  Filler, // 👈
} from "chart.js";

Chart.register(
  LineElement,
  PointElement,
  LineController,
  CategoryScale,
  LinearScale,
  Tooltip,
  Legend,
  Filler // 👈
);

// ===============
// Types & helpers
// ===============

type TelemetryKey = (typeof registry.fields)[number]["id"];

// ✅ Grupos (ordenados desde el registry)
const GROUPS_ORDER = registry.groupsOrder;

// ✅ Catálogo de campos para la UI
const FIELDS_CATALOG: Array<{
  key: TelemetryKey;
  label: string;
  group: string;
  default?: boolean;
}> = registry.fields.map((f) => ({
  key: f.id as TelemetryKey,
  label: f.label,
  group: f.group,
  default: !!f.default,
}));

const loadLocal = <T,>(k: string, fallback: T): T => {
  if (typeof window === "undefined") return fallback;
  try {
    const raw = localStorage.getItem(k);
    if (!raw) return fallback;
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
};

const saveLocal = (k: string, v: unknown) => {
  try {
    localStorage.setItem(k, JSON.stringify(v));
  } catch (error) {
    console.error("Error saving to localStorage:", error);
  }
};

export default function TelemetryLoggerSettings() {
  // ==================
  // Estado principal
  // ==================
  const [mass, setMass] = useState<number>(loadLocal("mass", 1.1));
  const [armLength, setArmLength] = useState<number>(
    loadLocal("armLength", 0.223)
  );

  // Qué campos se guardan
  const defaultSelected = useMemo(
    () =>
      loadLocal<TelemetryKey[]>("selectedFields", [])?.length
        ? loadLocal<TelemetryKey[]>("selectedFields", [])
        : registry.fields
            .filter((f) => f.default)
            .map((f) => f.id as TelemetryKey),
    []
  );
  const [selected, setSelected] = useState<TelemetryKey[]>(defaultSelected);

  // Política de retención: indefinida por defecto
  const [retentionMode, setRetentionMode] = useState<"infinite" | "ttl">(
    loadLocal("retentionMode", "infinite")
  );
  const [retentionUnit, setRetentionUnit] = useState<
    "minutes" | "hours" | "days"
  >(loadLocal("retentionUnit", "hours"));
  const [retentionValue, setRetentionValue] = useState<number>(
    loadLocal("retentionValue", 6)
  );

  // Disparador por throttle
  const [throttleMin, setThrottleMin] = useState<number>(
    loadLocal("throttleMin", 1200)
  );
  const [throttleMax, setThrottleMax] = useState<number>(
    loadLocal("throttleMax", 2000)
  );
  const [stopAfterSec, setStopAfterSec] = useState<number>(
    loadLocal("stopAfterSec", 5)
  );

  // Estado de grabación
  const [recording, setRecording] = useState(false);
  const [flightId, setFlightId] = useState<string | null>(null);
  const [applying, setApplying] = useState(false);
  const [serverMsg, setServerMsg] = useState<string | null>(null);

  // Persistencia en localStorage
  useEffect(() => saveLocal("mass", mass), [mass]);
  useEffect(() => saveLocal("armLength", armLength), [armLength]);
  useEffect(() => saveLocal("selectedFields", selected), [selected]);
  useEffect(() => saveLocal("retentionMode", retentionMode), [retentionMode]);
  useEffect(() => saveLocal("retentionUnit", retentionUnit), [retentionUnit]);
  useEffect(
    () => saveLocal("retentionValue", retentionValue),
    [retentionValue]
  );
  useEffect(() => saveLocal("throttleMin", throttleMin), [throttleMin]);
  useEffect(() => saveLocal("throttleMax", throttleMax), [throttleMax]);
  useEffect(() => saveLocal("stopAfterSec", stopAfterSec), [stopAfterSec]);

  // ==========
  // Derivados
  // ==========
  const retentionSeconds = useMemo(() => {
    if (retentionMode === "infinite") return undefined;
    const v = Math.max(1, retentionValue);
    if (retentionUnit === "minutes") return v * 60;
    if (retentionUnit === "hours") return v * 3600;
    return v * 86400; // days
  }, [retentionMode, retentionUnit, retentionValue]);

  const loggerConfig = useMemo(
    () => ({
      schemaVersion: registry.version,
      selectedFields: selected,
      retention:
        retentionMode === "infinite"
          ? { mode: "infinite" }
          : { mode: "ttl", seconds: retentionSeconds! },
      triggers: {
        startWhen: {
          key: "InputThrottle",
          between: [throttleMin, throttleMax],
        },
        stopWhen: {
          key: "InputThrottle",
          range: [throttleMin, throttleMax],
          outsideForSeconds: stopAfterSec,
        },
      },
      metadata: {
        mass,
        armLength,
        timeField: "time",
        modeField: "modo",
      },
    }),
    [
      selected,
      retentionMode,
      retentionSeconds,
      throttleMin,
      throttleMax,
      stopAfterSec,
      mass,
      armLength,
    ]
  );

  const applyConfig = async () => {
    setApplying(true);
    setServerMsg(null);
    try {
      console.log("Sending config:", JSON.stringify(loggerConfig, null, 2));
      const res = await fetch("http://localhost:3000/api/config", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(loggerConfig),
      });

      if (!res.ok) {
        const errorText = await res.text();
        console.error("Server error response:", res.status, errorText);
        throw new Error(
          `HTTP error! status: ${res.status}, body: ${errorText}`
        );
      }

      const data = (await res.json()) as { status?: string; flightId?: string };
      console.log("Config applied successfully:", data);
      setServerMsg(`Configuración aplicada (${data?.status ?? "ok"})`);
    } catch (error) {
      console.error("Failed to apply configuration:", error);
      setServerMsg(
        `Error: ${error instanceof Error ? error.message : "Unknown error"}`
      );
    } finally {
      setApplying(false);
    }
  };

  const startRecording = async () => {
    if (recording) return;
    setRecording(true);
    setServerMsg(null);
    try {
      console.log(
        "Starting recording with config:",
        JSON.stringify(loggerConfig, null, 2)
      );
      const res = await fetch("http://localhost:3000/api/start", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(loggerConfig),
      });

      if (!res.ok) {
        const errorText = await res.text();
        console.error("Start recording error:", res.status, errorText);
        throw new Error(
          `HTTP error! status: ${res.status}, body: ${errorText}`
        );
      }

      const data = (await res.json()) as { status?: string; flightId?: string };
      console.log("Recording started:", data);
      setFlightId(data?.flightId ?? null);
      setServerMsg(
        `Grabación iniciada${
          data?.flightId ? ` (flightId: ${data.flightId})` : ""
        }`
      );
    } catch (error) {
      console.error("Failed to start recording:", error);
      setServerMsg(
        `Error: ${error instanceof Error ? error.message : "Unknown error"}`
      );
      setRecording(false);
    }
  };

  const stopRecording = async () => {
    if (!recording) return;
    try {
      await fetch("http://localhost:3000/api/stop", { method: "POST" });
      setServerMsg("Grabación detenida");
    } catch {
      setServerMsg("No se pudo detener la grabación (se detendrá localmente)");
    } finally {
      setRecording(false);
      setFlightId(null);
    }
  };

  // =====================
  // UI utilities
  // =====================
  const toggleKey = (k: TelemetryKey) =>
    setSelected((prev) =>
      prev.includes(k) ? prev.filter((x) => x !== k) : [...prev, k]
    );

  const selectPreset = (preset: "basico" | "motores" | "actitud") => {
    if (preset === "basico")
      setSelected([
        "InputThrottle",
        "AngleRoll",
        "AnglePitch",
        "RateRoll",
        "RatePitch",
        "RateYaw",
      ] as TelemetryKey[]);

    if (preset === "motores")
      setSelected([
        "MotorInput1",
        "MotorInput2",
        "MotorInput3",
        "MotorInput4",
      ] as TelemetryKey[]);

    if (preset === "actitud")
      setSelected([
        "AngleRoll",
        "AnglePitch",
        "Yaw",
        "AngleRoll_est",
        "KalmanAnglePitch",
      ] as TelemetryKey[]);
  };
  // =====================
  // Render
  // =====================
  return (
    <div className="p-6 text-white max-w-5xl mx-auto space-y-8">
      <h1 className="text-3xl font-bold text-center">
        ⚙️ Configuración de Registro (Logger)
      </h1>

      {/* Datos físicos (opcionales) */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        <div className="bg-gray-800 p-5 rounded-2xl shadow-lg">
          <h2 className="text-xl font-semibold mb-4 border-b border-gray-700 pb-2">
            ✈️ Datos de Vuelo
          </h2>
          <div className="space-y-4">
            <div>
              <label className="block mb-2 font-medium">Masa (kg)</label>
              <input
                type="number"
                step="0.01"
                value={mass}
                onChange={(e) => setMass(parseFloat(e.target.value))}
                className="w-full p-3 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
              />
            </div>
            <div>
              <label className="block mb-2 font-medium">
                Longitud del brazo (m)
              </label>
              <input
                type="number"
                step="0.001"
                value={armLength}
                onChange={(e) => setArmLength(parseFloat(e.target.value))}
                className="w-full p-3 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
              />
            </div>
          </div>
        </div>

        {/* Retención + Trigger */}
        <div className="bg-gray-800 p-5 rounded-2xl shadow-lg space-y-4">
          <h2 className="text-xl font-semibold border-b border-gray-700 pb-2">
            🗂️ Retención & Trigger
          </h2>

          {/* Retención */}
          <div>
            <div className="font-medium mb-2">Tiempo de guardado</div>
            <div className="flex flex-wrap items-center gap-3">
              <label className="inline-flex items-center gap-2">
                <input
                  type="radio"
                  className="accent-green-500"
                  checked={retentionMode === "infinite"}
                  onChange={() => setRetentionMode("infinite")}
                />
                <span>Indefinido</span>
              </label>
              <label className="inline-flex items-center gap-2">
                <input
                  type="radio"
                  className="accent-green-500"
                  checked={retentionMode === "ttl"}
                  onChange={() => setRetentionMode("ttl")}
                />
                <span>Limitar a</span>
              </label>
              {retentionMode === "ttl" && (
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    min={1}
                    value={retentionValue}
                    onChange={(e) =>
                      setRetentionValue(parseInt(e.target.value || "1"))
                    }
                    className="w-24 p-2 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
                  />
                  <select
                    value={retentionUnit}
                    onChange={(e: React.ChangeEvent<HTMLSelectElement>) =>
                      setRetentionUnit(
                        e.target.value as "minutes" | "hours" | "days"
                      )
                    }
                    className="p-2 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
                  >
                    <option value="minutes">minutos</option>
                    <option value="hours">horas</option>
                    <option value="days">días</option>
                  </select>
                </div>
              )}
            </div>
          </div>

          {/* Trigger por throttle */}
          <div className="mt-3">
            <div className="font-medium mb-2">
              Disparador de vuelo por{" "}
              <code className="bg-gray-700 px-1 rounded">InputThrottle</code>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="block text-sm text-gray-300">Mínimo</label>
                <input
                  type="number"
                  value={throttleMin}
                  onChange={(e) =>
                    setThrottleMin(parseInt(e.target.value || "0"))
                  }
                  className="w-full p-2 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
                />
              </div>
              <div>
                <label className="block text-sm text-gray-300">Máximo</label>
                <input
                  type="number"
                  value={throttleMax}
                  onChange={(e) =>
                    setThrottleMax(parseInt(e.target.value || "0"))
                  }
                  className="w-full p-2 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
                />
              </div>
              <div className="col-span-2">
                <label className="block text-sm text-gray-300">
                  Terminar vuelo si sale del rango durante (s)
                </label>
                <input
                  type="number"
                  min={1}
                  value={stopAfterSec}
                  onChange={(e) =>
                    setStopAfterSec(parseInt(e.target.value || "1"))
                  }
                  className="w-full p-2 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
                />
              </div>
            </div>
            <p className="text-xs text-gray-400 mt-2">
              Solo se segmentarán y guardarán <b>vuelos</b> cuando el throttle
              esté entre {throttleMin} y {throttleMax}. Fuera de ese rango, el
              logger puede ignorar o bufferizar, según tu backend.
            </p>
          </div>
        </div>
      </div>

      {/* Selector de campos con chips animadas */}
      <div className="bg-gray-800 p-5 rounded-2xl shadow-lg">
        <div className="flex items-center justify-between">
          <h2 className="text-xl font-semibold border-b border-gray-700 pb-2">
            📡 Campos a guardar
          </h2>
          <div className="flex items-center gap-2">
            <button
              onClick={() => selectPreset("basico")}
              className="px-3 py-1.5 rounded bg-gray-700 hover:bg-gray-600 text-sm"
            >
              Preset básico
            </button>
            <button
              onClick={() => selectPreset("actitud")}
              className="px-3 py-1.5 rounded bg-gray-700 hover:bg-gray-600 text-sm"
            >
              Actitud
            </button>
            <button
              onClick={() => selectPreset("motores")}
              className="px-3 py-1.5 rounded bg-gray-700 hover:bg-gray-600 text-sm"
            >
              Motores
            </button>
          </div>
        </div>

        {GROUPS_ORDER.map((g) => {
          const items = FIELDS_CATALOG.filter((f) => f.group === g);
          if (!items.length) return null;
          return (
            <div key={g} className="mt-4">
              <div className="text-sm uppercase tracking-wider text-gray-400 mb-2">
                {g}
              </div>
              <div className="flex flex-wrap gap-2">
                {items.map((f) => {
                  const active = selected.includes(f.key);
                  return (
                    <motion.button
                      key={String(f.key)}
                      onClick={() => toggleKey(f.key)}
                      whileTap={{ scale: 0.95 }}
                      animate={{ opacity: 1 }}
                      initial={{ opacity: 0 }}
                      className={`px-3 py-1.5 rounded-full border text-sm transition ${
                        active
                          ? "bg-green-600/20 border-green-500 text-green-200"
                          : "bg-gray-700/40 border-gray-600 text-gray-200 hover:bg-gray-700"
                      }`}
                      title={
                        active ? "Quitar del registro" : "Añadir al registro"
                      }
                    >
                      <span className="inline-block w-1.5 h-1.5 rounded-full mr-2 bg-current" />
                      {f.label}
                      <AnimatePresence>
                        {active && (
                          <motion.span
                            initial={{ scale: 0, opacity: 0 }}
                            animate={{ scale: 1, opacity: 1 }}
                            exit={{ scale: 0, opacity: 0 }}
                            className="ml-2 text-xs"
                          >
                            ✓
                          </motion.span>
                        )}
                      </AnimatePresence>
                    </motion.button>
                  );
                })}
              </div>
            </div>
          );
        })}

        {/* Seleccionadas resumen */}
        <div className="mt-5 text-sm text-gray-300">
          <span className="font-medium">
            Seleccionadas ({selected.length}):
          </span>
          <div className="mt-2 flex flex-wrap gap-2">
            {selected.map((k) => (
              <span
                key={String(k)}
                className="px-2 py-1 rounded bg-gray-700 text-gray-100 text-xs border border-gray-600"
              >
                {String(k)}
              </span>
            ))}
          </div>
        </div>
      </div>

      {/* Acciones */}
      <div className="flex flex-wrap items-center gap-3">
        <button
          onClick={applyConfig}
          disabled={applying}
          className={`px-4 py-2 rounded-lg font-semibold shadow ${
            applying
              ? "bg-gray-600 cursor-not-allowed"
              : "bg-blue-600 hover:bg-blue-700"
          }`}
        >
          Aplicar configuración
        </button>

        {!recording ? (
          <button
            onClick={startRecording}
            className="px-4 py-2 rounded-lg font-semibold shadow bg-green-600 hover:bg-green-700"
          >
            🎥 Iniciar grabación
          </button>
        ) : (
          <button
            onClick={stopRecording}
            className="px-4 py-2 rounded-lg font-semibold shadow bg-red-600 hover:bg-red-700"
          >
            ⏹️ Detener
          </button>
        )}

        {flightId && (
          <span className="text-sm text-gray-300">
            flightId:{" "}
            <code className="bg-gray-800 px-2 py-1 rounded">{flightId}</code>
          </span>
        )}

        {serverMsg && (
          <span className="text-sm text-gray-400">{serverMsg}</span>
        )}
      </div>

      {/* Previsualización del datos, tablas graficas, metricas de datos de vuelo */}
      <div className="bg-gray-900 p-4 rounded-xl border border-gray-800 text-sm overflow-x-auto"></div>
    </div>
  );
}
