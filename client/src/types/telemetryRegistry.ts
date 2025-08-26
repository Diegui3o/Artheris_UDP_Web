export type FieldType = "float" | "int" | "string" | "bool";

export interface TelemetryField {
  // id canónico y ESTABLE para DB/ML
  id: string; // p.ej. "AngleRoll"
  // alias de entrada aceptados desde el firmware/front (evita roturas)
  sourceKeys: string[]; // p.ej. ["AngleRoll", "roll", "AngleRoll_est"]
  label: string; // etiqueta UI
  group: string; // para UI (Actitud, Rates, etc.)
  unit?: string; // p.ej. "deg"
  type: FieldType;
  default?: boolean; // si aparece seleccionado por defecto en UI
}

export const registry = {
  version: 1 as const,
  measurement: "telemetry" as const, // medición/tabla en QuestDB
  groupsOrder: [
    "Actitud",
    "Rates",
    "IMU",
    "Entradas",
    "Motores",
    "Errores",
    "Otros",
    "Gains",
  ],
  // ⚠️ Mantén ids estables. Si cambias un id, crea uno nuevo y depreca el anterior.
  fields: [
    // Actitud
    {
      id: "AngleRoll",
      sourceKeys: ["AngleRoll", "roll"],
      label: "AngleRoll",
      group: "Actitud",
      unit: "deg",
      type: "float",
      default: true,
    },
    {
      id: "AnglePitch",
      sourceKeys: ["AnglePitch", "pitch"],
      label: "AnglePitch",
      group: "Actitud",
      unit: "deg",
      type: "float",
      default: true,
    },
    {
      id: "Yaw",
      sourceKeys: ["yaw", "AngleYaw"],
      label: "Yaw",
      group: "Actitud",
      unit: "deg",
      type: "float",
    },
    {
      id: "AngleRoll_est",
      sourceKeys: ["AngleRoll_est"],
      label: "AngleRoll_est",
      group: "Actitud",
      unit: "deg",
      type: "float",
    },
    {
      id: "KalmanAnglePitch",
      sourceKeys: ["KalmanAnglePitch"],
      label: "KalmanAnglePitch",
      group: "Actitud",
      unit: "deg",
      type: "float",
    },

    // Rates
    { id: "RateRoll", sourceKeys: ["RateRoll", "gyroRateRoll"], label: "RateRoll", group: "Rates", unit: "dps", type: "float" },
    { id: "RatePitch", sourceKeys: ["RatePitch", "gyroRatePitch"], label: "RatePitch", group: "Rates", unit: "dps", type: "float" },
    { id: "RateYaw", sourceKeys: ["RateYaw"], label: "RateYaw", group: "Rates", unit: "dps", type: "float" },

    // IMU
    { id: "GyroXdps", sourceKeys: ["GyroXdps"], label: "Gyro X dps", group: "IMU", unit: "dps", type: "float" },
    { id: "GyroYdps", sourceKeys: ["GyroYdps"], label: "Gyro Y dps", group: "IMU", unit: "dps", type: "float" },
    { id: "GyroZdps", sourceKeys: ["GyroZdps"], label: "Gyro Z dps", group: "IMU", unit: "dps", type: "float" },

    // Entradas
    { id: "InputThrottle", sourceKeys: ["InputThrottle"], label: "InputThrottle", group: "Entradas", type: "int", default: true },
    { id: "InputRoll", sourceKeys: ["InputRoll"], label: "InputRoll", group: "Entradas", type: "int" },
    { id: "InputPitch", sourceKeys: ["InputPitch"], label: "InputPitch", group: "Entradas", type: "int" },
    { id: "InputYaw", sourceKeys: ["InputYaw"], label: "InputYaw", group: "Entradas", type: "int" },

    // Motores
    { id: "MotorInput1", sourceKeys: ["MotorInput1"], label: "Motor 1", group: "Motores", type: "int" },
    { id: "MotorInput2", sourceKeys: ["MotorInput2"], label: "Motor 2", group: "Motores", type: "int" },
    { id: "MotorInput3", sourceKeys: ["MotorInput3"], label: "Motor 3", group: "Motores", type: "int" },
    { id: "MotorInput4", sourceKeys: ["MotorInput4"], label: "Motor 4", group: "Motores", type: "int" },

    // Errores
    { id: "error_phi", sourceKeys: ["error_phi", "ErrorRoll"], label: "error_phi", group: "Errores", unit: "deg", type: "float" },
    { id: "error_theta", sourceKeys: ["error_theta", "ErrorPitch"], label: "error_theta", group: "Errores", unit: "deg", type: "float" },
    { id: "ErrorYaw", sourceKeys: ["ErrorYaw"], label: "ErrorYaw", group: "Errores", unit: "deg", type: "float" },

    // Otros
    { id: "Altura", sourceKeys: ["Altura"], label: "Altura", group: "Otros", unit: "m", type: "float" },
    { id: "tau_x", sourceKeys: ["tau_x"], label: "tau_x", group: "Otros", unit: "N·m", type: "float" },
    { id: "tau_y", sourceKeys: ["tau_y"], label: "tau_y", group: "Otros", unit: "N·m", type: "float" },
    { id: "tau_z", sourceKeys: ["tau_z"], label: "tau_z", group: "Otros", unit: "N·m", type: "float" },

    // Gains (opcionales)
    { id: "Kc", sourceKeys: ["Kc"], label: "Kc", group: "Gains", type: "float" },
    { id: "Ki", sourceKeys: ["Ki"], label: "Ki", group: "Gains", type: "float" },

    // Modo (string)
    { id: "modo", sourceKeys: ["modo", "modoActual"], label: "Modo", group: "Otros", type: "string" },

    // Timestamps externos (en caso lleguen)
    { id: "time", sourceKeys: ["time"], label: "time", group: "Otros", type: "string" },
  ] satisfies TelemetryField[],
} as const;

export type TelemetryKey = typeof registry.fields[number]["id"];

export type GenericRecord = Record<string, unknown>;
export function normalizeRecord(raw: GenericRecord) {
  const out: Partial<Record<TelemetryKey, string | number | boolean>> = {};
  for (const f of registry.fields) {
    for (const alias of f.sourceKeys) {
      if (Object.prototype.hasOwnProperty.call(raw, alias)) {
        const v = (raw as Record<string, unknown>)[alias];
        if (v !== undefined && v !== null && v !== "") {
          out[f.id as TelemetryKey] = v as string | number | boolean;
          break;
        }
      }
    }
  }
  return out;
}

function escapeTagOrMeasurement(s: string) {
  return s.replace(/[, =]/g, (m) => ({ ",": "\\,", " ": "\\ ", "=": "\\=" }[m]!));
}

function escapeFieldString(s: string) {
  return s.replace(/"/g, '\\"');
}

/**
 * Serializa a Influx/QuestDB Line Protocol
 * @param measurement nombre de la medición (tabla)
 * @param tags tags indexed (string)
 * @param fields campos (numéricos, booleanos o string)
 * @param tsNs timestamp en nanosegundos (opcional). Si no se provee, QuestDB usará server time
 */
export function toLineProtocol(
  measurement: string,
  tags: Record<string, string | number | boolean | undefined>,
  fields: Record<string, string | number | boolean>,
  tsNs?: number
) {
  const m = escapeTagOrMeasurement(measurement);
  const t = Object.entries(tags)
    .filter(([, v]) => v !== undefined && v !== null && v !== "")
    .map(([k, v]) => `${escapeTagOrMeasurement(k)}=${escapeTagOrMeasurement(String(v))}`)
    .join(",");

  const f = Object.entries(fields)
    .map(([k, v]) => {
      if (typeof v === "string") return `${k}="${escapeFieldString(v)}"`;
      if (typeof v === "boolean") return `${k}=${v ? "t" : "f"}`;
      if (Number.isInteger(v as number)) return `${k}=${v}i`;
      return `${k}=${v}`; // float
    })
    .join(",");

  const head = t ? `${m},${t}` : m;
  return tsNs ? `${head} ${f} ${tsNs}` : `${head} ${f}`;
}

/**
 * Convierte un AnglesData-like en una línea LP estable, usando ids canónicos.
 */
export function toQuestDBLine(
  raw: GenericRecord,
  opts: {
    tags?: Record<string, string | number | boolean | undefined>;
    // si el raw trae "time" en ms/iso, conviértelo a ns aquí si quieres
    timestampNs?: number;
  } = {}
) {
  const norm = normalizeRecord(raw);
  // Separa strings/bools de numéricos para fields
  const numFields: Record<string, number> = {};
  const otherFields: Record<string, string | number | boolean> = {};

  for (const f of registry.fields) {
    const k = f.id as TelemetryKey;
    const v = norm[k];
    if (v === undefined) continue;
    if (f.type === "string" || f.type === "bool") {
otherFields[k] = v;
    } else if (typeof v === "number") {
      numFields[k] = v;
    } else if (typeof v === "string") {
      otherFields[k] = v;
    }
  }

  const fields: Record<string, string | number | boolean> = { ...numFields, ...otherFields };
  return toLineProtocol(registry.measurement, opts.tags ?? {}, fields, opts.timestampNs);
}

export type AnglesDataCanonical = Partial<Record<TelemetryKey, number | string | boolean>> & {
};
