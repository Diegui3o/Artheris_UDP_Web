import { Canvas, useFrame, useLoader } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
import { OBJLoader } from "three/examples/jsm/loaders/OBJLoader.js";
import { useEffect, useRef, useMemo, useState } from "react";
import * as THREE from "three";
import { AnglesData } from "../types/angles";

interface DroneProps {
  latestAnglesRef: React.MutableRefObject<AnglesData>;
  latestKalmanRef: React.MutableRefObject<{ roll: number; pitch: number }>;
}

/* ---------- Helpers SIN any ---------- */
type AnyObj = Record<string, unknown>;

const isObj = (v: unknown): v is AnyObj => typeof v === "object" && v !== null;
const get = (o: unknown, k: string): unknown =>
  isObj(o) ? (o as AnyObj)[k] : undefined;

const looksLikeTelemetry = (o: unknown) =>
  isObj(o) &&
  ("AngleRoll" in o ||
    "AnglePitch" in o ||
    "AngleYaw" in o ||
    "roll" in o ||
    "pitch" in o ||
    "yaw" in o);

const toNum = (v: unknown, d = 0): number => {
  if (typeof v === "number") return v;
  if (typeof v === "string") {
    const n = Number(v);
    return Number.isFinite(n) ? n : d;
  }
  return d;
};

/** Normaliza distintos esquemas a AnglesData con AngleRoll/AnglePitch/AngleYaw numéricos */
function normalizeAngles(raw: unknown): AnglesData | undefined {
  if (!isObj(raw)) return undefined;

  // raw.payload || raw.data || raw.body || raw
  const p1 = get(raw, "payload") ?? get(raw, "data") ?? get(raw, "body") ?? raw;
  const p2 = get(p1, "payload") ?? p1;

  if (!looksLikeTelemetry(p2)) return undefined;

  const AngleRoll = toNum(get(p2, "AngleRoll"), toNum(get(p2, "roll")));
  const AnglePitch = toNum(get(p2, "AnglePitch"), toNum(get(p2, "pitch")));
  const AngleYaw = toNum(get(p2, "AngleYaw"), toNum(get(p2, "yaw")));

  return {
    ...(isObj(p2) ? p2 : {}),
    AngleRoll,
    AnglePitch,
    AngleYaw,
    AngleRoll_est: toNum(get(p2, "AngleRoll_est"), AngleRoll),
    AnglePitch_est: toNum(get(p2, "AnglePitch_est"), AnglePitch),
  } as AnglesData;
}

/* ---------- Componente hijo: SIN WebSocket aquí ---------- */
function Drone({ latestAnglesRef, latestKalmanRef }: DroneProps) {
  const droneRef = useRef<THREE.Group>(null);

  const targetEuler = useMemo(() => new THREE.Euler(0, 0, 0, "YXZ"), []);
  const targetQuat = useMemo(() => new THREE.Quaternion(), []);
  const tmpQuat = useMemo(() => new THREE.Quaternion(), []);

  useEffect(() => {
    const socket = new WebSocket("ws://localhost:9001");

    let msgCount = 0;
    let unrecognizedLogged = false;

    socket.onmessage = (event) => {
      try {
        const raw = JSON.parse(event.data);

        if (msgCount < 3) {
          // loggea solo los 3 primeros crudos
          console.debug("[WS raw]", raw);
        }

        const norm = normalizeAngles(raw);
        if (!norm) {
          if (!unrecognizedLogged) {
            console.warn("[WS] Formato no reconocido. Ejemplo:", raw);
            unrecognizedLogged = true;
          }
          return;
        }

        if (msgCount < 3) {
          // loggea normalizados 3 primeras veces
          console.debug("[WS norm]", {
            AngleRoll: norm.AngleRoll,
            AnglePitch: norm.AnglePitch,
            AngleYaw: norm.AngleYaw,
            AngleRoll_est: norm.AngleRoll_est,
            AnglePitch_est: norm.AnglePitch_est,
          });
        }

        latestAnglesRef.current = norm;
        latestKalmanRef.current.roll =
          norm.AngleRoll_est ?? norm.AngleRoll ?? 0;
        latestKalmanRef.current.pitch =
          norm.AnglePitch_est ?? norm.AnglePitch ?? 0;

        msgCount++;
      } catch (e) {
        console.error("❌ Error procesando WS:", e);
      }
    };

    socket.onopen = () => console.info("[WS] Conectado");
    socket.onerror = (e) => console.error("[WS] Error", e);
    socket.onclose = () => console.info("[WS] Cerrado");

    return () => socket.close();
  }, [latestAnglesRef, latestKalmanRef]);

  useFrame((_, delta) => {
    const g = droneRef.current;
    if (!g) return;

    const data = latestAnglesRef.current;

    const pitchDeg = data.AnglePitch ?? 0;
    const yawDeg = data.AngleYaw ?? 0;
    const rollDeg = data.AngleRoll ?? 0;

    targetEuler.set(
      THREE.MathUtils.degToRad(pitchDeg),
      THREE.MathUtils.degToRad(yawDeg),
      THREE.MathUtils.degToRad(rollDeg)
    );
    targetQuat.setFromEuler(targetEuler);

    const k = 120; // antes 20
    const t = 1 - Math.exp(-k * delta);
    tmpQuat.copy(g.quaternion).slerp(targetQuat, t);
    g.quaternion.copy(tmpQuat);
    latestKalmanRef.current.roll = data.AngleRoll_est ?? rollDeg;
    latestKalmanRef.current.pitch = data.AnglePitch_est ?? pitchDeg;
  });

  const obj = useLoader(OBJLoader, "/src/models/base(2).obj");

  return (
    <primitive ref={droneRef} object={obj} scale={1} position={[0, 0, 0]} />
  );
}

/* ---------- Componente padre: único WebSocket + deps correctas ---------- */
export default function Dron3D() {
  const latestAnglesRef = useRef<AnglesData>({});
  const latestKalmanRef = useRef({ roll: 0, pitch: 0 });

  // 👇 estado solo para refrescar HUD
  const [hud, setHud] = useState({ roll: 0, pitch: 0, yaw: 0 });

  useEffect(() => {
    // refresca HUD a ~10 Hz
    const id = setInterval(() => {
      const a = latestAnglesRef.current;
      setHud({
        roll: a.AngleRoll ?? 0,
        pitch: a.AnglePitch ?? 0,
        yaw: a.AngleYaw ?? 0,
      });
    }, 100);
    return () => clearInterval(id);
  }, []);

  const hudStyle: React.CSSProperties = {
    position: "absolute",
    top: 10,
    left: 10,
    backgroundColor: "rgba(0, 0, 0, 0.34)",
    color: "#0AC4ff",
    padding: 15,
    borderRadius: 10,
    fontFamily: "monospace",
    boxShadow: "0 4px 8px rgba(0, 0, 0, 0.23)",
  };

  return (
    <div style={{ position: "relative", width: "70vw", height: "70vh" }}>
      <div style={hudStyle}>
        <p style={{ margin: 0, fontSize: 16, fontWeight: "bold" }}>
          Kalman Roll: {hud.roll.toFixed(2)}°
        </p>
        <p style={{ margin: 0, fontSize: 16, fontWeight: "bold" }}>
          Kalman Pitch: {hud.pitch.toFixed(2)}°
        </p>
      </div>

      <Canvas camera={{ position: [0, 1, 2.9], fov: 50 }}>
        <ambientLight intensity={0.5} />
        <directionalLight position={[5, 10, 5]} intensity={1.2} />
        <axesHelper args={[0.4]} />
        <gridHelper args={[10, 10]} />
        <Drone
          latestAnglesRef={latestAnglesRef}
          latestKalmanRef={latestKalmanRef}
        />
        <OrbitControls />
      </Canvas>
    </div>
  );
}
