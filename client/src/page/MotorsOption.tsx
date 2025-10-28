import { useState, useEffect, useCallback, useMemo } from "react";
import { FaPlus } from "react-icons/fa";
import { useWebSocket } from "../hooks/useWebSocket";
import { AnimatedBackground } from "../components/AnimatedBackground";
import { MotorCard } from "../components/MotorCard";

type MotorKey = `motor${number}`;
type OneMotor = { on: boolean; speed: number }; // µs (1000–2000)
type MotorState = Record<MotorKey, OneMotor>;

// === Ajusta estos valores a tu setup ===
const DEFAULT_MOTORS = 4; // arranca mostrando 4 motores
const DEFAULT_IDLE_US = 1170; // idle real
const MIN_US = 1000;
const MAX_US = 2000;

// Debounce genérico por tuplas (compatible browser/Node)
function debounce<A extends unknown[]>(
  fn: (...args: A) => void,
  ms: number
): (...args: A) => void {
  let t: ReturnType<typeof setTimeout>;
  return (...args: A) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...args), ms);
  };
}

// Lo que el ESP32 espera DENTRO de "payload" del WS: { motor: {...} } o { motors: {...} }
type MotorPayload =
  | { motor: { id: number; speed: number } }
  | { motor: { id: number; state: boolean } }
  | { motors: { speed: number } }
  | { motors: { state: boolean } }
  | { motors: { ids: number[]; speed: number } }
  | { motors: { ids: number[]; state: boolean } };

export default function MotorControl() {
  const { isConnected, sendMessage } = useWebSocket("ws://localhost:9001");

  // Estado: on+speed por motor
  const [activeMotors, setActiveMotors] = useState<number>(DEFAULT_MOTORS);
  const [motorStatus, setMotorStatus] = useState<MotorState>(() => {
    const init = {} as MotorState;
    for (let i = 1; i <= DEFAULT_MOTORS; i++) {
      init[`motor${i}` as MotorKey] = { on: false, speed: DEFAULT_IDLE_US };
    }
    return init;
  });

  // Velocidad global (para “todos a X µs”)
  const [globalSpeed, setGlobalSpeed] = useState<number>(DEFAULT_IDLE_US);

  // Counter to force re-renders when motor states change
  const [globalTick, setGlobalTick] = useState<number>(0);

  // Helper: envía { type:"command", payload: <MotorPayload> }
  const sendCmd = useCallback(
    (payload: MotorPayload) => {
      if (!isConnected) return;
      sendMessage("command", payload);
    },
    [isConnected, sendMessage]
  );

  // === Acciones por motor ===
  const setOneMotorOn = useCallback(
    (id: number, speed?: number) => {
      const key = `motor${id}` as MotorKey;
      const us = Math.max(
        MIN_US,
        Math.min(MAX_US, speed ?? motorStatus[key]?.speed ?? DEFAULT_IDLE_US)
      );

      // UI optimista
      setMotorStatus((prev) => ({
        ...prev,
        [key]: { on: true, speed: us },
      }));

      // Update globalTick to force re-render
      setGlobalTick((prev) => prev + 1);

      // Firmware: {"type":"command","payload":{"motor":{"id":N,"speed":US}}}
      sendCmd({ motor: { id, speed: us } });
    },
    [sendCmd, motorStatus]
  );

  const setOneMotorOff = useCallback(
    (id: number) => {
      const key = `motor${id}` as MotorKey;
      setMotorStatus((prev) => ({
        ...prev,
        [key]: { on: false, speed: prev[key]?.speed ?? DEFAULT_IDLE_US },
      }));

      // Update globalTick to force re-render
      setGlobalTick((prev) => prev + 1);

      // Firmware: {"type":"command","payload":{"motor":{"id":N,"state":false}}}
      sendCmd({ motor: { id, state: false } });
    },
    [sendCmd]
  );

  const toggleMotor = useCallback(
    (id: number, newOn: boolean) => {
      if (newOn) setOneMotorOn(id);
      else setOneMotorOff(id);
    },
    [setOneMotorOn, setOneMotorOff]
  );

  // Slider por motor (debounced) — usa tupla [id, us]
  const sendSpeedDebounced = useMemo(
    () =>
      debounce(([id, us]: [number, number]) => {
        const key = `motor${id}` as MotorKey;
        // Solo enviar si está conectado y el motor está encendido
        if (isConnected && motorStatus[key]?.on) {
          sendCmd({ motor: { id, speed: us } });
        }
      }, 100), // Tiempo de debounce reducido para mejor respuesta
    [sendCmd, motorStatus, isConnected]
  );

  const handleSpeedChange = useCallback(
    (id: number, us: number) => {
      const key = `motor${id}` as MotorKey;
      const newSpeed = Math.max(MIN_US, Math.min(MAX_US, us));
      
      // Actualizar el estado de la UI inmediatamente
      setMotorStatus(prev => ({
        ...prev,
        [key]: { 
          ...prev[key], 
          speed: newSpeed 
        }
      }));

      // Si el motor está encendido, enviar el comando (con debounce)
      if (motorStatus[key]?.on) {
        sendSpeedDebounced([id, newSpeed]);
      }
    },
    [motorStatus, sendSpeedDebounced]
  );

  // === Acciones globales ===
  const turnAllOn = useCallback(() => {
    for (let i = 1; i <= activeMotors; i++) {
      const key = `motor${i}` as MotorKey;
      const us = motorStatus[key]?.speed ?? DEFAULT_IDLE_US;
      setTimeout(() => setOneMotorOn(i, us), (i - 1) * 70);
    }
  }, [activeMotors, motorStatus, setOneMotorOn]);

  // Reemplaza turnAllOff con esta:
  const turnAllOff = useCallback(() => {
    for (let i = 1; i <= activeMotors; i++) {
      setTimeout(() => setOneMotorOff(i), (i - 1) * 50);
    }
  }, [activeMotors, setOneMotorOff]);

  const setAllToSpeed = useCallback(
    (us: number) => {
      const v = Math.max(MIN_US, Math.min(MAX_US, us));
      setGlobalSpeed(v);

      const ids: number[] = [];
      for (let i = 1; i <= activeMotors; i++) {
        const k = `motor${i}` as MotorKey;
        if (motorStatus[k]?.on) ids.push(i);
      }
      if (ids.length) sendCmd({ motors: { ids, speed: v } });

      // UI: no cambies 'on', solo la velocidad
      setMotorStatus((prev) => {
        const next: MotorState = { ...prev };
        for (let i = 1; i <= activeMotors; i++) {
          const k = `motor${i}` as MotorKey;
          const wasOn = prev[k]?.on ?? false;
          next[k] = { on: wasOn, speed: v }; // ✅ respeta ON/OFF
        }
        return next;
      });

      setGlobalTick((prev) => prev + 1);
    },
    [activeMotors, motorStatus, sendCmd]
  );

  // === Añadir / Quitar motores ===
  const addMotor = useCallback(() => {
    const n = activeMotors + 1;
    setActiveMotors(n);
    setMotorStatus((prev) => ({
      ...prev,
      [`motor${n}`]: { on: false, speed: DEFAULT_IDLE_US },
    }));
  }, [activeMotors]);

  const removeMotor = useCallback(() => {
    if (activeMotors <= 1) return;
    const id = activeMotors;
    const key = `motor${id}` as MotorKey;
    const wasOn = motorStatus[key]?.on;

    setActiveMotors((p) => p - 1);
    // Evita no-unused-vars usando delete
    setMotorStatus((prev) => {
      const rest = { ...prev };
      delete rest[key];
      return rest as MotorState;
    });

    if (wasOn) sendCmd({ motor: { id, state: false } });
  }, [activeMotors, motorStatus, sendCmd]);

  // Estado conexión
  useEffect(() => {
    if (!isConnected) {
      console.log("Intentando conectar con el servidor WebSocket...");
    }
  }, [isConnected]);

  // UI helpers (colores)
  const getMotorColor = useCallback((motorNum: number, opacity = 1): string => {
    const colors = [
      "rgba(99,102,241,{o})", // Indigo
      "rgba(139,92,246,{o})", // Violet
      "rgba(236,72,153,{o})", // Pink
      "rgba(249,115,22,{o})", // Orange
      "rgba(16,185,129,{o})", // Emerald
      "rgba(234,179,8,{o})", // Yellow
    ];
    return colors[(motorNum - 1) % colors.length].replace(
      "{o}",
      String(opacity)
    );
  }, []);

  return (
    <div className="relative">
      <AnimatedBackground />

      <div className="flex flex-col w-full space-y-4">
        {/* Header */}
        <div className="w-full">
          <div className="flex flex-col md:flex-row md:items-center md:justify-between gap-4">
            <div>
              <h1 className="text-3xl font-bold bg-gradient-to-r from-blue-400 to-cyan-300 bg-clip-text text-transparent">
                Control de Motores
              </h1>
              <p className="text-gray-400 mt-1">
                Gestiona los motores del sistema
              </p>
            </div>
            <div
              className={`flex items-center gap-2 px-4 py-2 rounded-full ${
                isConnected
                  ? "bg-green-900/30 text-green-400"
                  : "bg-red-900/30 text-red-400"
              }`}
            >
              <div
                className={`w-2.5 h-2.5 rounded-full ${
                  isConnected ? "bg-green-500" : "bg-red-500"
                } animate-pulse`}
              />
              <span className="text-sm font-medium">
                {isConnected ? "Conectado" : "Desconectado"}
              </span>
            </div>
          </div>
        </div>

        {/* Control general */}
        <div className="w-full space-y-3">
          <div className="flex flex-col md:flex-row md:items-center md:justify-between gap-4">
            <div>
              <h2 className="text-xl font-semibold text-white">
                Control General
              </h2>
              <p className="text-gray-400 text-sm">
                Gestiona todos los motores simultáneamente
              </p>
            </div>
            <div className="flex flex-wrap gap-3">
              <button
                onClick={turnAllOn}
                disabled={!isConnected}
                className="px-5 py-2.5 bg-blue-600 hover:bg-blue-700 text-white rounded-lg font-medium disabled:opacity-50"
              >
                Encender todos
              </button>
              <button
                onClick={turnAllOff}
                disabled={!isConnected}
                className="px-5 py-2.5 bg-yellow-600 hover:bg-yellow-700 text-white rounded-lg font-medium disabled:opacity-50"
              >
                Apagar todos
              </button>
            </div>
          </div>

          {/* Enhanced Global Speed Control */}
          <div className="rounded-xl p-4 bg-gradient-to-br from-black/20 to-black/40 border border-white/5 backdrop-blur-sm shadow-lg">
            <div className="flex items-center justify-between mb-3">
              <div className="flex items-center gap-2">
                <svg className="w-4 h-4 text-white/70" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
                </svg>
                <span className="text-white/90 text-sm font-medium">Velocidad Global</span>
              </div>
              <span className="text-white/90 font-mono text-sm px-2 py-1 bg-white/5 rounded-md">
                {globalSpeed} µs
              </span>
            </div>

            <div className="relative w-full py-6 px-2">
              <div className="relative w-full h-2 bg-gray-700 rounded-full">
                {/* Background track */}
                <div className="absolute inset-0 rounded-full bg-gray-700"></div>
                
                {/* Filled track */}
                <div 
                  className="absolute left-0 top-0 h-full rounded-full"
                  style={{
                    width: `${((globalSpeed - MIN_US) / (MAX_US - MIN_US)) * 100}%`,
                    background: 'linear-gradient(90deg, rgba(99,102,241,0.7), rgba(99,102,241,1))',
                  }}
                />
                
                {/* Hidden input that's actually interactive */}
                <input
                  type="range"
                  min={MIN_US}
                  max={MAX_US}
                  step={1}
                  value={globalSpeed}
                  onChange={(e) => setAllToSpeed(Number(e.target.value))}
                  disabled={!isConnected}
                  className="absolute inset-0 w-full h-full opacity-0 cursor-pointer z-10"
                />
                
                {/* Custom thumb */}
                <div 
                  className="absolute top-1/2 h-4 w-4 -mt-2 -ml-2 rounded-full bg-white shadow-lg transition-transform duration-150 hover:scale-125 z-20"
                  style={{
                    left: `${((globalSpeed - MIN_US) / (MAX_US - MIN_US)) * 100}%`,
                    transform: 'translateY(-50%)',
                    boxShadow: 'rgba(99, 102, 241, 0.5) 0px 0px 0px 4px',
                    pointerEvents: 'none' // Prevent the thumb from blocking the input
                  }}
                />
              </div>
            </div>

            {/* Min/Max labels */}
            <div className="flex items-center justify-between mt-2 px-1">
              <span className="text-xs text-white/60">{MIN_US} µs</span>
              <span className="text-[11px] text-white/60 italic">
                Aplica a todos los motores encendidos
              </span>
              <span className="text-xs text-white/60">{MAX_US} µs</span>
            </div>
          </div>
        </div>

        {/* Controles individuales */}
        <div className="w-full">
          {/* (Opcional) Título y contador */}
          <div className="flex justify-between items-center mb-3">
            <h2 className="text-lg font-semibold text-white">Motores</h2>
            <span className="text-xs text-gray-400">
              {activeMotors} motores
            </span>
          </div>

          {/* Cards */}
          <div className="grid grid-cols-1 gap-4 w-full">
            {Array.from({ length: activeMotors }).map((_, index) => {
              const motorNum = index + 1;
              const key = `motor${motorNum}` as MotorKey;
              const st = motorStatus[key] ?? {
                on: false,
                speed: DEFAULT_IDLE_US,
              };
              return (
                <MotorCard
                  key={key}
                  id={motorNum}
                  isOn={st.on}
                  speed={st.speed}
                  min={MIN_US}
                  max={MAX_US}
                  color={getMotorColor}
                  onToggle={toggleMotor}
                  onSpeed={handleSpeedChange}
                  disabled={!isConnected}
                  globalTick={globalTick}
                />
              );
            })}
          </div>

          {/* Añadir/Quitar motores */}
          <div className="flex gap-2 mt-3">
            <button
              onClick={addMotor}
              disabled={!isConnected}
              className="flex-1 py-1.5 text-xs bg-blue-600/80 hover:bg-blue-700 border border-blue-500/50 rounded flex items-center justify-center gap-1 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <FaPlus className="text-xs" />
              Añadir motor
            </button>
            {activeMotors > 1 && (
              <button
                onClick={removeMotor}
                disabled={!isConnected}
                className="flex-1 py-1.5 text-xs bg-red-600/80 hover:bg-red-700 border border-red-500/50 rounded flex items-center justify-center gap-1 disabled:opacity-50 disabled:cursor-not-allowed"
              >
                Quitar motor
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
