import { useEffect, useState } from "react";
import { Card, CardContent } from "../components/ui/Card";
import "./AnglesDisplay.css";
import { AnglesData } from "../types/angles";

// 📦 Componente para secciones
const Section = ({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) => (
  <div className="section">
    <h3 className="section-title">{title}</h3>
    <div>{children}</div>
  </div>
);

type AnyObj = Record<string, unknown>;
const isObj = (v: unknown): v is AnyObj => typeof v === "object" && v !== null;

const num = (v: unknown, d = undefined as number | undefined) =>
  typeof v === "number"
    ? v
    : typeof v === "string" && v.trim() !== ""
    ? Number(v)
    : d;
const str = (v: unknown) => (typeof v === "string" ? v : undefined);

function normalizeTelemetry(raw: unknown): AnglesData {
  if (!isObj(raw)) return {};

  // Ángulos canónicos (con alias)
  const AngleRoll_est = num(raw.AngleRoll_est ?? raw.roll);
  const AnglePitch_est = num(raw.AnglePitch_est ?? raw.pitch);
  const AngleYaw = num(raw.AngleYaw ?? raw.yaw);

  // Estimados / Kalman
  const AngleRoll = num(raw.AngleRoll ?? raw.KalmanAngleRoll);
  const AnglePitch = num(raw.AnglePitch ?? raw.KalmanAnglePitch);

  // Velocidades angulares (alias comunes)
  const RateRoll = num(
    raw.RateRoll ?? raw.GyroXdps ?? (raw as AnyObj).gyroRateRoll
  );
  const RatePitch = num(
    raw.RatePitch ?? raw.GyroYdps ?? (raw as AnyObj).gyroRatePitch
  );
  const RateYaw = num(
    raw.RateYaw ?? raw.GyroZdps ?? (raw as AnyObj).gyroRateYaw
  );

  // Acelerómetro (si llega con otro naming)
  const AccX = num((raw as AnyObj).AccX ?? (raw as AnyObj).accX);
  const AccY = num((raw as AnyObj).AccY ?? (raw as AnyObj).accY);
  const AccZ = num((raw as AnyObj).AccZ ?? (raw as AnyObj).accZ);

  // Entradas
  const InputThrottle = num(
    (raw as AnyObj).InputThrottle ?? (raw as AnyObj).throttle
  );
  const InputRoll = num((raw as AnyObj).InputRoll);
  const InputPitch = num((raw as AnyObj).InputPitch);
  const InputYaw = num((raw as AnyObj).InputYaw);

  // Motores
  const MotorInput1 = num(
    (raw as AnyObj).MotorInput1 ?? (raw as AnyObj).motor1 ?? 1000
  );
  const MotorInput2 = num(
    (raw as AnyObj).MotorInput2 ?? (raw as AnyObj).motor2 ?? 1000
  );
  const MotorInput3 = num(
    (raw as AnyObj).MotorInput3 ?? (raw as AnyObj).motor3 ?? 1000
  );
  const MotorInput4 = num(
    (raw as AnyObj).MotorInput4 ?? (raw as AnyObj).motor4 ?? 1000
  );

  // Torques y errores
  const tau_x = num((raw as AnyObj).tau_x);
  const tau_y = num((raw as AnyObj).tau_y);
  const tau_z = num((raw as AnyObj).tau_z);
  const error_phi = num((raw as AnyObj).error_phi ?? (raw as AnyObj).err_roll);
  const error_theta = num(
    (raw as AnyObj).error_theta ?? (raw as AnyObj).err_pitch
  );

  // Deseados
  const DesiredAngleRoll = num(
    (raw as AnyObj).DesiredAngleRoll ?? (raw as AnyObj).desiredRoll
  );
  const DesiredAnglePitch = num(
    (raw as AnyObj).DesiredAnglePitch ?? (raw as AnyObj).desiredPitch
  );
  const DesiredRateYaw = num(
    (raw as AnyObj).DesiredRateYaw ?? (raw as AnyObj).desiredYawRate
  );

  // Otros
  const Altura = num((raw as AnyObj).Altura ?? (raw as AnyObj).altitude);
  const modo = str((raw as AnyObj).modo) ?? str((raw as AnyObj).mode);
  const modoActual =
    str((raw as AnyObj).modoActual) ?? str((raw as AnyObj).currentMode);
  const k1 = num((raw as AnyObj).k1);
  const k2 = num((raw as AnyObj).k2);
  const k3 = num((raw as AnyObj).k3);
  const g1 = num((raw as AnyObj).g1);
  const g2 = num((raw as AnyObj).g2);
  const g3 = num((raw as AnyObj).g3);
  const m1 = num((raw as AnyObj).m1);
  const m2 = num((raw as AnyObj).m2);
  const m3 = num((raw as AnyObj).m3);

  return {
    AngleRoll,
    AnglePitch,
    AngleYaw,
    AngleRoll_est,
    AnglePitch_est,
    RateRoll,
    RatePitch,
    RateYaw,
    AccX,
    AccY,
    AccZ,
    InputThrottle,
    InputRoll,
    InputPitch,
    InputYaw,
    MotorInput1,
    MotorInput2,
    MotorInput3,
    MotorInput4,
    tau_x,
    tau_y,
    tau_z,
    error_phi,
    error_theta,
    DesiredAngleRoll,
    DesiredAnglePitch,
    DesiredRateYaw,
    Altura,
    modo,
    modoActual,
    k1,
    k2,
    k3,
    g1,
    g2,
    g3,
    m1,
    m2,
    m3,
  };
}

// 📦 Componente para mostrar campos
const Field = ({ label, value }: { label: string; value: unknown }) => (
  <p className="label-text">
    {label}:{" "}
    <span className="value-text">
      {typeof value === "number"
        ? value.toFixed(3)
        : value === undefined || value === null
        ? "-"
        : typeof value === "object"
        ? JSON.stringify(value)
        : String(value)}
    </span>
  </p>
);

// 📦 Componente principal
const DroneAngles = () => {
  const [angles, setAngles] = useState<AnglesData>({});

  useEffect(() => {
    const socket = new WebSocket("ws://localhost:9001");

    socket.onopen = () => {
      console.log("✅ Conectado al servidor WebSocket");
    };

    socket.onerror = (error) => {
      console.error("❌ Error en WebSocket:", error);
    };

    socket.onclose = () => {
      console.log("🔌 Conexión cerrada con el servidor");
    };

    socket.onmessage = (event) => {
      try {
        const data = JSON.parse(event.data);

        let telemetryData: AnglesData | undefined;

        if (data.type === "telemetry" && data.payload) {
          telemetryData = normalizeTelemetry(data.payload);
        } else {
          telemetryData = normalizeTelemetry(data);
        }

        if (telemetryData && Object.keys(telemetryData).length > 0) {
          setAngles((prev) => ({
            ...prev,
            ...telemetryData,
          }));
        }
      } catch (err) {
        console.error("❌ Error procesando WS:", err);
      }
    };
    return () => {
      console.log("🔌 Cerrando WebSocket...");
      socket.close();
    };
  }, []);

  return (
    <Card className="p-4 shadow-lg rounded-lg bg-black neon-card">
      <CardContent>
        <h2 className="main-title">Drone Telemetría</h2>

        <Section title="Ángulos">
          <Field label="AngleRoll" value={angles.AngleRoll_est} />
          <Field label="AnglePitch" value={angles.AnglePitch_est} />
          <Field label="AngleYaw" value={angles.AngleYaw} />
        </Section>

        <Section title="Kalman y Estimaciones">
          <Field label="Kalman AngleRoll" value={angles.AngleRoll} />
          <Field label="Kalman AnglePitch" value={angles.AnglePitch} />
        </Section>

        <Section title="Velocidades Angulares">
          <Field label="Rate Roll" value={angles.RateRoll} />
          <Field label="Rate Pitch" value={angles.RatePitch} />
          <Field label="Rate Yaw" value={angles.RateYaw} />
        </Section>

        <Section title="Valores Deseados">
          <Field label="Desired Roll" value={angles.DesiredAngleRoll} />
          <Field label="Desired Pitch" value={angles.DesiredAnglePitch} />
          <Field label="Desired Rate Yaw" value={angles.DesiredRateYaw} />
        </Section>

        <Section title="Acelerómetro">
          <Field label="Acc X" value={angles.AccX} />
          <Field label="Acc Y" value={angles.AccY} />
          <Field label="Acc Z" value={angles.AccZ} />
        </Section>

        <Section title="Errores y Torques">
          <Field label="Error phi" value={angles.error_phi} />
          <Field label="Error theta" value={angles.error_theta} />
          <Field label="Tau X" value={angles.tau_x} />
          <Field label="Tau Y" value={angles.tau_y} />
          <Field label="Tau Z" value={angles.tau_z} />
        </Section>

        <Section title="Entradas de Control">
          <Field label="InputThrottle" value={angles.InputThrottle} />
          <Field label="InputRoll" value={angles.InputRoll} />
          <Field label="InputPitch" value={angles.InputPitch} />
          <Field label="InputYaw" value={angles.InputYaw} />
        </Section>

        <Section title="Motores">
          <Field label="Motor 1" value={angles.MotorInput1} />
          <Field label="Motor 2" value={angles.MotorInput2} />
          <Field label="Motor 3" value={angles.MotorInput3} />
          <Field label="Motor 4" value={angles.MotorInput4} />
        </Section>

        <Section title="Valores de la matriz">
          <Field label="Ganancia k1" value={angles.k1} />
          <Field label="Ganancia k2" value={angles.k2} />
          <Field label="Ganancia k3" value={angles.k3} />
          <Field label="Ganancia g1" value={angles.g1} />
          <Field label="Ganancia g2" value={angles.g2} />
          <Field label="Ganancia g3" value={angles.g3} />
          <Field label="Ganancia m1" value={angles.m1} />
          <Field label="Ganancia m2" value={angles.m2} />
          <Field label="Ganancia m3" value={angles.m3} />
        </Section>

        <Section title="Otros">
          <Field label="Altura" value={angles.Altura} />
          <Field label="Modo" value={angles.modoActual} />
        </Section>
      </CardContent>
    </Card>
  );
};

export default DroneAngles;
