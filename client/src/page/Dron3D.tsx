import { Canvas, useFrame, useLoader } from "@react-three/fiber";
import { OrbitControls } from "@react-three/drei";
import { OBJLoader } from "three/examples/jsm/loaders/OBJLoader.js";
import React, { useEffect, useRef, useMemo, useState } from "react";
import * as THREE from "three";
import { AnglesData } from "../types/angles";

/* ---------- Helpers ---------- */
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

/** Normaliza distintos esquemas a AnglesData con AngleRoll/AnglePitch/Yaw numéricos */
function normalizeAngles(raw: unknown): AnglesData | undefined {
  if (!isObj(raw)) return undefined;
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

/* ---------- Componente hijo: sin WS ---------- */
function Drone({
  latestAnglesRef,
  latestKalmanRef,
}: {
  latestAnglesRef: React.MutableRefObject<AnglesData>;
  latestKalmanRef: React.MutableRefObject<{ roll: number; pitch: number }>;
}) {
  const droneRef = useRef<THREE.Group>(null);
  const targetEuler = useMemo(() => new THREE.Euler(0, 0, 0, "YXZ"), []);
  const targetQuat = useMemo(() => new THREE.Quaternion(), []);
  const tmpQuat = useMemo(() => new THREE.Quaternion(), []);

  const obj = useLoader(OBJLoader, "/src/models/base(2).obj");

  useFrame(() => {
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

    // Suavizado fijo por frame (estable)
    const alphaPerFrame = 0.3; // 0..1 (sube si quieres más “pegado”)
    tmpQuat.copy(g.quaternion).slerp(targetQuat, alphaPerFrame);
    g.quaternion.copy(tmpQuat);

    // HUD refs (por si los usas)
    latestKalmanRef.current.roll = data.AngleRoll_est ?? rollDeg;
    latestKalmanRef.current.pitch = data.AnglePitch_est ?? pitchDeg;
  });

  return (
    <primitive ref={droneRef} object={obj} scale={1} position={[0, 0, 0]} />
  );
}

/* ---------- Componente padre: WebSocket único + coalescing + HUD ---------- */
export default function Dron3D() {
  const latestAnglesRef = useRef<AnglesData>({});
  const latestKalmanRef = useRef({ roll: 0, pitch: 0 });
  const [hud, setHud] = useState({ roll: 0, pitch: 0 });

  // Coalesce: guardo solo el último mensaje recibido entre frames
  const lastMsgRef = useRef<AnglesData | null>(null);

  // WebSocket único aquí
  useEffect(() => {
    const socket = new WebSocket("ws://localhost:9001");
    let warned = false;

    socket.onmessage = (event) => {
      try {
        const norm = normalizeAngles(JSON.parse(event.data));
        if (norm) {
          lastMsgRef.current = norm;
        } else if (!warned) {
          console.warn(
            "[WS] Formato no reconocido (solo se muestra una vez). Ej:",
            event.data
          );
          warned = true;
        }
      } catch (e) {
        console.error("❌ WS parse:", e);
      }
    };

    socket.onopen = () => console.info("[WS] Conectado");
    socket.onerror = (e) => console.error("[WS] Error", e);
    socket.onclose = () => console.info("[WS] Cerrado");

    return () => socket.close();
  }, []);

  // Loop: aplicar último mensaje a refs una vez por frame
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const m = lastMsgRef.current;
      if (m) {
        latestAnglesRef.current = m;
        latestKalmanRef.current.roll = m.AngleRoll_est ?? m.AngleRoll ?? 0;
        latestKalmanRef.current.pitch = m.AnglePitch_est ?? m.AnglePitch ?? 0;
        // refresco HUD ligero (opcional)
        setHud({
          roll: latestKalmanRef.current.roll,
          pitch: latestKalmanRef.current.pitch,
        });
        lastMsgRef.current = null;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
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

      <Canvas
        camera={{ position: [0, 1, 2.9], fov: 50 }}
        dpr={[1, 1.5]}
        gl={{ antialias: false, powerPreference: "high-performance" }}
        shadows={false}
      >
        <ambientLight intensity={0.5} />
        <directionalLight position={[5, 10, 5]} intensity={1.2} />
        {/* helpers quítalos en prod si quieres más FPS */}
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
