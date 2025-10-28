// components/MotorCard.tsx
import {
  motion,
  useMotionValue,
  useSpring,
  useTransform,
  AnimatePresence,
  Variants,
} from "framer-motion";
import { useEffect, useMemo, useState } from "react";

type Props = {
  id: number;
  isOn: boolean;
  speed: number;
  min: number;
  max: number;
  color: (n: number, opacity?: number) => string;
  onToggle: (id: number, on: boolean) => void;
  onSpeed: (id: number, us: number) => void;
  disabled?: boolean;
  globalTick?: number; // 👈 se incrementa desde el padre al mover la velocidad global
};

export function MotorCard({
  id,
  isOn,
  speed,
  min,
  max,
  color,
  onToggle,
  onSpeed,
  disabled,
  globalTick,
}: Props) {
  // Progreso (0..1) con spring suave
  const progress = useMotionValue((speed - min) / (max - min));
  const smooth = useSpring(progress, {
    stiffness: 180,
    damping: 24,
    mass: 0.6,
  });

  // Glow según progreso
  const glowStrength = useTransform(smooth, [0, 1], [0, 1]);

  // Ancho animado de la barra

  // Color base (reutilizado)
  const base = useMemo(() => color(id, 0.9), [id, color]);

  // Flash global cuando sube globalTick
  const [flash, setFlash] = useState(false);
  useEffect(() => {
    if (globalTick === undefined) return;
    setFlash(true);
    const t = setTimeout(() => setFlash(false), 220);
    return () => clearTimeout(t);
  }, [globalTick]);

  useEffect(() => {
    progress.set((speed - min) / (max - min));
  }, [speed, min, max, progress]);

  // Variants para ON/OFF
  const cardVariants: Variants = {
    off: {
      scale: 1,
      boxShadow: "0 0 0px rgba(0,0,0,0)",
    },
    on: {
      scale: 1.02,
      boxShadow: `0 10px 30px ${color(id, 0.25)}`,
      transition: {
        type: "spring",
        stiffness: 160,
        damping: 20,
      },
    },
  };

  const glowShadow = useTransform(
    glowStrength,
    (g) => `0 0 ${10 + g * 25}px ${color(id, 0.35)}`
  );

  return (
    <motion.div
      className="relative rounded-xl p-4 transition-all duration-300 w-full min-h-[120px] flex flex-row items-center gap-6 border"
      style={{
        backgroundColor: isOn ? color(id, 0.9) : "rgba(55,65,81,0.8)",
        borderColor: isOn ? color(id, 0.4) : "rgba(75,85,99,0.3)",
      }}
      variants={cardVariants}
      initial={isOn ? "on" : "off"}
      animate={isOn ? "on" : "off"}
    >
      {/* Glow animado extra */}
      <motion.div
        className="absolute inset-0 rounded-xl pointer-events-none"
        style={{ boxShadow: glowShadow }}
      />

      {/* Flash cuando cambia la velocidad global */}
      <AnimatePresence>
        {flash && (
          <motion.div
            className="absolute inset-0 rounded-xl pointer-events-none"
            initial={{ opacity: 0 }}
            animate={{ opacity: 0.25 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.22 }}
            style={{
              background: `radial-gradient(1200px 400px at 50% 50%, ${base}, transparent)`,
            }}
          />
        )}
      </AnimatePresence>

      <div className="flex-1 relative z-10">
        {/* Título + LED */}
        <div className="flex items-center justify-between mb-3">
          <span className="text-lg font-semibold text-white drop-shadow-sm">
            Motor {id}
          </span>

          {/* LED latido */}
          <motion.span
            className="w-3 h-3 rounded-full shadow-sm"
            style={{
              backgroundColor: isOn ? color(id, 1) : "rgb(248,113,113)",
            }}
            animate={
              isOn
                ? {
                    scale: [1, 1.25, 1],
                    filter: [
                      "brightness(1)",
                      "brightness(1.5)",
                      "brightness(1)",
                    ],
                  }
                : { scale: 1, filter: "brightness(1)" }
            }
            transition={
              isOn
                ? { repeat: Infinity, duration: 1.2, ease: "easeInOut" }
                : undefined
            }
          />
        </div>

        {/* Enhanced Speed Control */}
        <div className="rounded-xl p-4 bg-gradient-to-br from-black/20 to-black/40 border border-white/5 backdrop-blur-sm mb-4 shadow-lg">
          <div className="flex items-center justify-between mb-3">
            <div className="flex items-center gap-2">
              <svg
                className="w-4 h-4 text-white/70"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M13 10V3L4 14h7v7l9-11h-7z"
                />
              </svg>
              <span className="text-white/90 text-sm font-medium">
                Velocidad
              </span>
            </div>
            <motion.span
              className="text-white/90 font-mono text-sm px-2 py-1 bg-white/5 rounded-md"
              initial={{ scale: 1 }}
              animate={{ scale: flash ? [1, 1.1, 1] : 1 }}
              transition={{ duration: 0.3 }}
            >
              {speed} µs
            </motion.span>
          </div>

          {/* Single Slider with Custom Styling */}
          <div className="relative w-full py-6 px-2">
            <div className="relative w-full h-2 bg-gray-700 rounded-full">
              {/* Background track */}
              <div className="absolute inset-0 rounded-full bg-gray-700"></div>
              
              {/* Filled track */}
              <div 
                className="absolute left-0 top-0 h-full rounded-full"
                style={{
                  width: `${((speed - min) / (max - min)) * 100}%`,
                  background: `linear-gradient(90deg, ${color(id, 0.7)}, ${color(id, 1)})`,
                }}
              />
              
              {/* Hidden input that's actually interactive */}
              <input
                type="range"
                min={min}
                max={max}
                step={1}
                value={speed}
                onChange={(e) => onSpeed(id, Number(e.target.value))}
                disabled={disabled}
                className="absolute inset-0 w-full h-full opacity-0 cursor-pointer z-10"
              />
              
              {/* Custom thumb */}
              <div 
                className="absolute top-1/2 h-4 w-4 -mt-2 -ml-2 rounded-full bg-white shadow-lg transition-transform duration-150 hover:scale-125 z-20"
                style={{
                  left: `${((speed - min) / (max - min)) * 100}%`,
                  transform: 'translateY(-50%)',
                  boxShadow: `${color(id, 0.5)} 0px 0px 0px 4px`,
                  pointerEvents: 'none' // Prevent the thumb from blocking the input
                }}
              />
            </div>
          </div>

          {/* Status and min/max labels */}
          <div className="flex items-center justify-between mt-2 px-1">
            <span className="text-xs text-white/60">{min} µs</span>
            <span className="text-[11px] text-white/60 italic">
              {isOn ? "Ajustando en tiempo real..." : "Ajusta la velocidad"}
            </span>
            <span className="text-xs text-white/60">{max} µs</span>
          </div>
        </div>

        {/* Botones */}
        <div className="flex gap-3">
          <button
            onClick={() => onToggle(id, true)}
            className={`flex-1 py-2 px-3 text-sm rounded-lg font-medium transition-all min-w-[100px] ${
              isOn
                ? "bg-white/10 text-white/90 border border-white/20"
                : "bg-white/5 text-white/90 hover:bg-white/10 border border-white/10 hover:border-white/20"
            } ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
            disabled={disabled || isOn}
          >
            {isOn ? "Encendido" : "Encender"}
          </button>

          <button
            onClick={() => onToggle(id, false)}
            className={`flex-1 py-2 px-3 text-sm rounded-lg font-medium transition-all min-w-[100px] ${
              !isOn
                ? "bg-white/5 text-white/90 border border-white/10"
                : "bg-red-500/10 text-red-300 hover:bg-red-500/20 border border-red-500/20 hover:border-red-500/30"
            } ${disabled ? "opacity-50 cursor-not-allowed" : ""}`}
            disabled={disabled || !isOn}
          >
            Apagar
          </button>
        </div>
      </div>
    </motion.div>
  );
}
