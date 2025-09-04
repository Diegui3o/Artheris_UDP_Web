"use client";
import { registry } from "../types/telemetryRegistry";
import { useEffect, useMemo, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import FlightDashboard from "./FlightDashboard";
import SwitchControl from "./ModeSwitch";
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

type TelemetryKey = string;

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
  } catch (err) {
    console.error("Error saving to localStorage:", err);
  }
};

interface FieldDefinition {
  key: TelemetryKey;
  label: string;
  group: string;
  default: boolean;
}

const BASE_CATALOG: FieldDefinition[] = registry.fields.map((f) => ({
  key: f.id as TelemetryKey,
  label: f.label,
  group: f.group,
  default: !!f.default,
}));

const GROUPS_ORDER = [...registry.groupsOrder] as const;
const EXTRA_GROUP = "Otros";
const ORDERED_GROUPS = [...GROUPS_ORDER, EXTRA_GROUP];

const getGroupOrder = (group: string): number => {
  const idx = ORDERED_GROUPS.indexOf(group);
  return idx === -1 ? ORDERED_GROUPS.length : idx;
};

// Base URL for API requests
const API_BASE = import.meta.env.VITE_API_BASE || "http://localhost:3000";

const fetchAvailableFields = async (): Promise<string[]> => {
  const r = await fetch(`${API_BASE}/api/telemetry/fields`);
  if (!r.ok) throw new Error("Failed to fetch available fields");
  const data = (await r.json()) as { fields: string[]; last_updated: string };
  return data.fields;
};

export default function TelemetryLoggerSettings() {
  // Estado “físico” opcional
  const [mass, setMass] = useState<number>(loadLocal("mass", 1.1));
  const [armLength, setArmLength] = useState<number>(
    loadLocal("armLength", 0.223)
  );

  // Estado de campos disponibles (desde backend)
  const [availableFields, setAvailableFields] = useState<string[]>([]);

  // Estado de selección de campos
  const [selected, setSelected] = useState<TelemetryKey[]>(
    loadLocal<TelemetryKey[]>("selectedFields", [])
  );

  // Retención/trigger
  const [retentionMode, setRetentionMode] = useState<"infinite" | "ttl">(
    loadLocal("retentionMode", "infinite")
  );
  const [retentionValue, setRetentionValue] = useState<number>(
    loadLocal("retentionValue", 1)
  );
  const [retentionUnit, setRetentionUnit] = useState<
    "minutes" | "hours" | "days"
  >(loadLocal("retentionUnit", "days"));
  const [throttleMin, setThrottleMin] = useState<number>(
    loadLocal("throttleMin", 5)
  );
  const [throttleMax, setThrottleMax] = useState<number>(
    loadLocal("throttleMax", 100)
  );
  const [stopAfterSec, setStopAfterSec] = useState<number>(
    loadLocal("stopAfterSec", 5)
  );
  // === Select helpers ===
  const selectAllVisible = () => {
    const all = filteredCatalog.map((f) => f.key);
    setSelected(all);
    saveLocal("selectedFields", all);
    window.getSelection()?.removeAllRanges();
  };

  const clearAll = () => {
    setSelected([]);
    saveLocal("selectedFields", []);
    window.getSelection()?.removeAllRanges();
  };

  const invertVisible = () => {
    const all = filteredCatalog.map((f) => f.key);
    const cur = new Set(selected);
    const inverted = all.filter((k) => !cur.has(k));
    setSelected(inverted);
    saveLocal("selectedFields", inverted);
    window.getSelection()?.removeAllRanges();
  };

  // (Opcional) seleccionar/quitar por grupo
  const selectGroup = (group: string) => {
    const keys = (fieldsByGroup[group] ?? []).map((f) => f.key);
    const next = Array.from(new Set([...selected, ...keys]));
    setSelected(next);
    saveLocal("selectedFields", next);
    window.getSelection()?.removeAllRanges();
  };

  const clearGroup = (group: string) => {
    const keys = new Set((fieldsByGroup[group] ?? []).map((f) => f.key));
    const next = selected.filter((k) => !keys.has(k));
    setSelected(next);
    saveLocal("selectedFields", next);
    window.getSelection()?.removeAllRanges();
  };

  // Estado UI / server
  const [applying, setApplying] = useState(false);
  const [recording, setRecording] = useState(false);
  const [flightId, setFlightId] = useState<string | null>(null);
  const [serverMsg, setServerMsg] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    const load = async () => {
      const fields = await fetchAvailableFields();
      if (mounted) {
        setAvailableFields(fields);
      }
    };
    load();
    return () => {
      mounted = false;
    };
  }, []);

  // Refresco periódico SIEMPRE (cada 3s)
  useEffect(() => {
    const id = setInterval(async () => {
      try {
        const fields = await fetchAvailableFields();
        setAvailableFields(fields); // sin el guard de length
      } catch (e) {
        console.error(e);
      }
    }, 3000);
    return () => clearInterval(id);
  }, []);

  // =========================
  // Catálogo final a mostrar
  // =========================
  const filteredCatalog = useMemo(() => {
    if (availableFields.length === 0) {
      // Al inicio: mostrar solo los default del registry para no dejar vacío
      return BASE_CATALOG.filter((f) => f.default).sort((a, b) => {
        const ga = getGroupOrder(a.group);
        const gb = getGroupOrder(b.group);
        return ga !== gb ? ga - gb : a.label.localeCompare(b.label);
      });
    }

    const available = new Set(availableFields);
    const known = new Map(BASE_CATALOG.map((f) => [f.key, f]));

    // 1) Campos del registry que están llegando (o marcados default)
    const keepFromRegistry = BASE_CATALOG.filter(
      (f) => available.has(f.key) || f.default
    );

    // 2) Campos nuevos que llegan y no existen en registry
    const dynamicNew = [...available]
      .filter((k) => !known.has(k))
      .map((k) => ({
        key: k,
        label: k,
        group: EXTRA_GROUP,
        default: false,
      }));

    const all = [...keepFromRegistry, ...dynamicNew];

    all.sort((a, b) => {
      const ga = getGroupOrder(a.group);
      const gb = getGroupOrder(b.group);
      return ga !== gb ? ga - gb : a.label.localeCompare(b.label);
    });

    return all;
  }, [availableFields]);

  // Asegurar que la selección sea consistente con lo visible
  useEffect(() => {
    const visibleSet = new Set(filteredCatalog.map((f) => f.key));

    setSelected((prev) => {
      const filtered = prev.filter((k) => visibleSet.has(k));
      // Si no hay nada seleccionado y existen defaults visibles, usar esos
      if (filtered.length === 0) {
        const defaults = filteredCatalog
          .filter((f) => f.default)
          .map((f) => f.key);
        if (defaults.length) {
          saveLocal("selectedFields", defaults);
          return defaults;
        }
      }
      // Persistir
      if (filtered.length !== prev.length) {
        saveLocal("selectedFields", filtered);
      }
      return filtered;
    });
  }, [filteredCatalog]);

  // ======================
  // Agrupación para UI
  // ======================
  const fieldsByGroup = useMemo(() => {
    const groups: Record<string, typeof filteredCatalog> = {};
    for (const f of filteredCatalog) {
      if (!groups[f.group]) groups[f.group] = [];
      groups[f.group].push(f);
    }

    const sorted: Record<string, typeof filteredCatalog> = {};
    for (const g of ORDERED_GROUPS) {
      if (groups[g]?.length) sorted[g] = groups[g];
    }
    for (const g of Object.keys(groups)) {
      if (!sorted[g]) sorted[g] = groups[g];
    }
    return sorted;
  }, [filteredCatalog]);

  // ======================
  // Acciones
  // ======================
  const toggleKey = (key: TelemetryKey) => {
    try {
      setSelected((prev) => {
        const next = prev.includes(key)
          ? prev.filter((k) => k !== key)
          : [...prev, key];
        saveLocal("selectedFields", next);
        return next;
      });
    } catch (error) {
      console.error("Error in toggleKey:", error);
    }

    // Clear any existing selection to prevent getRangeAt errors
    const selection = window.getSelection();
    if (selection) {
      selection.removeAllRanges();
    }
  };

  const selectPreset = (preset: "basico" | "actitud" | "motores") => {
    try {
      let next: string[] = [];

      if (preset === "basico") {
        next = [
          "AngleRoll",
          "AnglePitch",
          "Yaw",
          "Altitude",
          "Speed",
          "BatteryVoltage",
          "RSSI",
        ];
      } else if (preset === "actitud") {
        next = [
          "AngleRoll",
          "AnglePitch",
          "Yaw",
          "GyroX",
          "GyroY",
          "GyroZ",
          "AccX",
          "AccY",
          "AccZ",
        ];
      } else if (preset === "motores") {
        next = [
          "Motor1",
          "Motor2",
          "Motor3",
          "Motor4",
          "MotorInput1",
          "MotorInput2",
          "MotorInput3",
          "MotorInput4",
          "MotorOutput1",
          "MotorOutput2",
          "MotorOutput3",
          "MotorOutput4",
          "BatteryVoltage",
        ];
      }

      // Filter out any fields that don't exist in the catalog
      const filteredNext = next.filter((k) =>
        filteredCatalog.some((f) => f.key === k)
      );

      setSelected(filteredNext);
      saveLocal("selectedFields", filteredNext);

      // Clear selection after changing preset
      const selection = window.getSelection();
      if (selection) {
        selection.removeAllRanges();
      }
    } catch (error) {
      console.error("Error in selectPreset:", error);
    }
  };

  const applyConfig = async () => {
    // Set applying state to true at the start
    setApplying(true);

    try {
      // Persistimos local
      saveLocal("mass", mass);
      saveLocal("armLength", armLength);
      saveLocal("selectedFields", selected);
      saveLocal("retentionMode", retentionMode);
      saveLocal("retentionUnit", retentionUnit);
      saveLocal("throttleMin", throttleMin);
      saveLocal("throttleMax", throttleMax);
      saveLocal("stopAfterSec", stopAfterSec);
      if (retentionMode === "ttl") saveLocal("retentionValue", retentionValue);

      // Prepare the configuration object matching the LoggerConfig struct
      const body = {
        schemaVersion: 1, // Required by LoggerConfig
        selectedFields: selected,
        retention:
          retentionMode === "infinite"
            ? { mode: "infinite" as const }
            : {
                mode: "ttl" as const,
                seconds:
                  retentionValue *
                  (retentionUnit === "minutes"
                    ? 60
                    : retentionUnit === "hours"
                    ? 3600
                    : 86400),
              },
        triggers: {
          // Arranca cuando el throttle supera el umbral mínimo
          startWhen: {
            key: "InputThrottle" as const,
            greater_than: throttleMin,
          },
          // Para cuando baja de ese mínimo durante N segundos
          ...(stopAfterSec > 0
            ? {
                stopWhen: {
                  key: "InputThrottle" as const,
                  less_than: throttleMin,
                  afterSeconds: stopAfterSec,
                },
              }
            : {}),
        },
        metadata: { mass, armLength },
      };

      const res = await fetch(`${API_BASE}/api/config`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });

      if (!res.ok) {
        const errorText = await res.text();
        throw new Error(`Server responded with ${res.status}: ${errorText}`);
      }

      setServerMsg("Configuración aplicada ✅");
    } catch (error) {
      console.error("Error applying configuration:", error);
      const message =
        error instanceof Error ? error.message : "Error desconocido";
      setServerMsg(`Error: ${message}`);
    } finally {
      // Always reset the applying state, even if there was an error
      setApplying(false);
    }
  };

  const startRecording = async () => {
    try {
      const config = {
        schemaVersion: 1,
        selectedFields: selected,
        retention: {
          mode: "infinite" as const,
        },
        triggers: {
          startWhen: {
            key: "InputThrottle" as const,
            greater_than: throttleMin,
          },
          ...(stopAfterSec > 0
            ? {
                stopWhen: {
                  key: "InputThrottle" as const,
                  less_than: throttleMin,
                  afterSeconds: stopAfterSec,
                },
              }
            : {}),
        },
        metadata: {
          mass: mass,
          armLength: armLength,
        },
      };

      // Make sure we have a clean selection state
      const selection = window.getSelection();
      if (selection) {
        selection.removeAllRanges();
      }

      const res = await fetch(`${API_BASE}/api/start`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(config),
      });

      if (!res.ok) {
        const errorData = await res.json().catch(() => ({}));
        throw new Error(errorData.message || "start failed");
      }

      const data = (await res.json()) as { status: string; flightId: string };
      setRecording(true);
      setFlightId(data.flightId);
      setServerMsg("Grabación iniciada 🎥");
    } catch (error) {
      console.error("Start recording error:", error);
      const message = error instanceof Error ? error.message : String(error);
      setServerMsg(`Error al iniciar: ${message}`);
    }
  };

  const stopRecording = async () => {
    try {
      const res = await fetch(`${API_BASE}/api/stop`, { method: "POST" });
      if (!res.ok) throw new Error("stop failed");
      setRecording(false);
      setServerMsg("Grabación detenida ⏹️");
    } catch (error) {
      console.error(error);
      const message = error instanceof Error ? error.message : String(error);
      setServerMsg(`Error al detener: ${message}`);
    }
  };

  // ======================
  // Render
  // ======================
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
                onChange={(e) => setMass(parseFloat(e.target.value) || 0)}
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
                onChange={(e) => setArmLength(parseFloat(e.target.value) || 0)}
                className="w-full p-3 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
              />
            </div>
            <SwitchControl />
          </div>
        </div>

        {/* Retención & Trigger */}
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
                      setRetentionValue(parseInt(e.target.value) || 1)
                    }
                    className="w-24 p-2 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
                  />
                  <select
                    value={retentionUnit}
                    onChange={(e) =>
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
          <div className="pt-2">
            <div className="font-medium mb-2">
              Disparador de vuelo por{" "}
              <code className="bg-gray-700 px-1 rounded">InputThrottle</code>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div>
                <label className="block text-sm text-gray-300 mb-1">
                  Mínimo
                </label>
                <input
                  type="number"
                  min={0}
                  max={100}
                  value={throttleMin}
                  onChange={(e) =>
                    setThrottleMin(parseInt(e.target.value) || 0)
                  }
                  className="w-full p-2 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
                />
              </div>
              <div>
                <label className="block text-sm text-gray-300 mb-1">
                  Máximo
                </label>
                <input
                  type="number"
                  min={0}
                  max={100}
                  value={throttleMax}
                  onChange={(e) =>
                    setThrottleMax(parseInt(e.target.value) || 0)
                  }
                  className="w-full p-2 rounded bg-gray-700 border border-gray-600 focus:outline-none focus:ring-2 focus:ring-green-500"
                />
              </div>
              <div className="col-span-2">
                <label className="block text-sm text-gray-300 mb-1">
                  Terminar vuelo si sale del rango durante (s)
                </label>
                <input
                  type="number"
                  min={1}
                  value={stopAfterSec}
                  onChange={(e) =>
                    setStopAfterSec(parseInt(e.target.value) || 1)
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

      {/* Selector de campos */}
      <div className="bg-gray-800 p-5 rounded-2xl shadow-lg">
        <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 mb-6">
          <h2 className="text-xl font-semibold">📡 Campos a guardar</h2>
          <div className="flex flex-wrap gap-2">
            <button
              onClick={() => selectPreset("basico")}
              className="px-3 py-1.5 rounded bg-gray-700 hover:bg-green-600/50 text-sm transition-colors"
            >
              Preset básico
            </button>
            <button
              onClick={selectAllVisible}
              className="px-3 py-1.5 rounded bg-gray-700 hover:bg-gray-600 text-sm"
            >
              Seleccionar todo
            </button>
            <button
              onClick={clearAll}
              className="px-3 py-1.5 rounded bg-gray-700 hover:bg-gray-600 text-sm"
            >
              Quitar todo
            </button>
            <button
              onClick={invertVisible}
              className="px-3 py-1.5 rounded bg-gray-700 hover:bg-gray-600 text-sm"
            >
              Invertir
            </button>

            <button
              onClick={() => selectPreset("actitud")}
              className="px-3 py-1.5 rounded bg-gray-700 hover:bg-blue-600/50 text-sm transition-colors"
            >
              Actitud
            </button>
            <button
              onClick={() => selectPreset("motores")}
              className="px-3 py-1.5 rounded bg-gray-700 hover:bg-yellow-600/50 text-sm transition-colors"
            >
              Motores
            </button>
          </div>
        </div>

        {/* Grupos + chips */}
        <div className="space-y-6">
          {Object.entries(fieldsByGroup).map(([group, fields]) => (
            <div key={group} className="mb-6">
              <div className="flex items-center gap-2 mb-2">
                <h3 className="text-lg font-medium text-gray-300">{group}</h3>
                <button
                  onClick={() => selectGroup(group)}
                  className="px-2 py-0.5 rounded bg-gray-700 hover:bg-gray-600 text-xs"
                >
                  Seleccionar grupo
                </button>
                <button
                  onClick={() => clearGroup(group)}
                  className="px-2 py-0.5 rounded bg-gray-700 hover:bg-gray-600 text-xs"
                >
                  Quitar grupo
                </button>
              </div>
              <div className="flex flex-wrap gap-2">
                {fields.map((field) => {
                  const active = selected.includes(field.key);
                  return (
                    <motion.button
                      key={field.key}
                      onClick={() => toggleKey(field.key)}
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
                      {field.label}
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
          ))}
        </div>

        {/* Resumen selección */}
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

      {/* Placeholder de preview / charts si luego quieres integrar */}
      <div className="bg-gray-900 p-4 rounded-xl border border-gray-800 text-sm overflow-x-auto">
        <FlightDashboard />
      </div>
    </div>
  );
}
