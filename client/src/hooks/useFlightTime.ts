import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import type { AnglesData } from "../types/angles";

export type FlightRecord = {
  startedAt: number;
  endedAt: number;
  durationSec: number;
  meanAbsRoll: number;
  meanAbsPitch: number;
};

// Claves de persistencia
const LS_TOTAL = "armeris:flight:totalSec";
const LS_LAST = "armeris:flight:lastSec";
const LS_HISTORY = "armeris:flight:history";

/**
 * Cuenta tiempo desde que InputThrottle >= 1100 (inicio)
 * hasta que InputThrottle >= 2000 (fin).
 * Persiste totales e historial en localStorage.
 */
export const useFlightTime = (sample?: AnglesData & { timestamp?: number }) => {
  // Umbrales (ajústalos si quieres)
  const THR_START = 1100; // inicio de conteo
  const THR_END   = 2000; // fin de conteo
  const MIN_SESSION_MS = 300; // filtra rebotes muy cortos

  // Estado público
  const [isFlying, setIsFlying] = useState(false);
  const [currentFlightSec, setCurrentFlightSec] = useState(0);

  const [totalFlightSec, setTotalFlightSec] = useState<number>(() => {
    const s = localStorage.getItem(LS_TOTAL);
    return s ? Number(s) || 0 : 0;
  });
  const [lastFlightSec, setLastFlightSec] = useState<number>(() => {
    const s = localStorage.getItem(LS_LAST);
    return s ? Number(s) || 0 : 0;
  });

  const [meanAbsRoll, setMeanAbsRoll] = useState(0);
  const [meanAbsPitch, setMeanAbsPitch] = useState(0);
  const [lastMeanAbsRoll, setLastMeanAbsRoll] = useState(0);
  const [lastMeanAbsPitch, setLastMeanAbsPitch] = useState(0);

  const [history, setHistory] = useState<FlightRecord[]>(() => {
    const s = localStorage.getItem(LS_HISTORY);
    if (!s) return [];
    try { return JSON.parse(s) as FlightRecord[]; } catch { return []; }
  });

  // Refs de la sesión
  const startTimeRef = useRef<number | null>(null);
  const prevTsRef = useRef<number | null>(null);
  const sumAbsRollRef = useRef(0);
  const sumAbsPitchRef = useRef(0);
  const countRef = useRef<number>(0);
  const lastThrottleValueRef = useRef<number>(0);
  const throttleTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [debouncedThrottle, setDebouncedThrottle] = useState<number>(0);

  // Throttle actual y etiqueta legible
  const throttleValue = sample?.InputThrottle ?? 0;

  // Efecto para debounce del throttle
  useEffect(() => {
    if (throttleTimeoutRef.current) {
      clearTimeout(throttleTimeoutRef.current);
    }
    
    throttleTimeoutRef.current = setTimeout(() => {
      // Solo actualizamos si hay un cambio significativo (más de 10 unidades)
      if (Math.abs((lastThrottleValueRef.current - throttleValue)) > 10) {
        lastThrottleValueRef.current = throttleValue;
        setDebouncedThrottle(throttleValue);
      }
    }, 50); // 50ms de debounce

    return () => {
      if (throttleTimeoutRef.current) {
        clearTimeout(throttleTimeoutRef.current);
      }
    };
  }, [throttleValue]);
  const throttleLabel = useMemo(() => {
    const thr = debouncedThrottle; // Usamos el valor debounceado
    if (thr >= THR_END) return "Máximo (FULL)";
    if (thr >= 1700)   return "Alto";
    if (thr >= 1300)   return "Medio";
    if (thr >= THR_START) return "Arranque/Despegue";
    if (thr > 0)       return "Bajo/Idle";
    return "—";
  }, [debouncedThrottle, THR_END, THR_START]);

  // Persistencia - Agrupado en un solo efecto para optimizar
  useEffect(() => {
    localStorage.setItem(LS_TOTAL, String(totalFlightSec));
    localStorage.setItem(LS_LAST, String(lastFlightSec));
    localStorage.setItem(LS_HISTORY, JSON.stringify(history.slice(-100)));
  }, [totalFlightSec, lastFlightSec, history]);

  // Lógica principal
  useEffect(() => {
    if (!sample) return; // No hacer nada si no hay muestra
    
    const ts = sample.timestamp ?? Date.now();
    const thr = debouncedThrottle; // Usamos el valor debounceado

    // Inicio de vuelo
    if (!isFlying && thr >= THR_START) {
      // Usar el callback de setState para evitar dependencias en el array
      setIsFlying(true);
      startTimeRef.current = ts;
      prevTsRef.current = ts;
      sumAbsRollRef.current = 0;
      sumAbsPitchRef.current = 0;
      countRef.current = 0;
      setCurrentFlightSec(0);
      setMeanAbsRoll(0);
      setMeanAbsPitch(0);
      return;
    }

    // Vuelo en curso
    if (isFlying) {
      // acumula tiempo aunque el throttle varie; sólo se cierra al alcanzar THR_END
      const prev = prevTsRef.current ?? ts;
      const dtSec = Math.max(0, (ts - prev) / 1000);
      prevTsRef.current = ts;

      // Usar el callback de setState para evitar dependencias en el array
      setCurrentFlightSec(s => s + dtSec);

      // acumula medias si hay ángulos
      const absR = Math.abs(sample.KalmanAngleRoll ?? 0);
      const absP = Math.abs(sample.KalmanAnglePitch ?? 0);
      sumAbsRollRef.current += absR;
      sumAbsPitchRef.current += absP;
      countRef.current += 1;

      const n = countRef.current || 1;
      const newMeanRoll = sumAbsRollRef.current / n;
      const newMeanPitch = sumAbsPitchRef.current / n;
      
      // Solo actualizar si hay un cambio significativo
      if (Math.abs(meanAbsRoll - newMeanRoll) > 0.01) {
        setMeanAbsRoll(newMeanRoll);
      }
      if (Math.abs(meanAbsPitch - newMeanPitch) > 0.01) {
        setMeanAbsPitch(newMeanPitch);
      }

      // ¿Fin de vuelo?
      if (thr <= THR_START) {
        const start = startTimeRef.current ?? ts;
        const durationMs = ts - start;

        if (durationMs >= MIN_SESSION_MS) {
          const durSec = durationMs / 1000;
          const nFinal = countRef.current || 1;
          const mRoll = sumAbsRollRef.current / nFinal;
          const mPitch = sumAbsPitchRef.current / nFinal;

          setLastFlightSec(durSec);
          setTotalFlightSec((t) => t + durSec);
          setLastMeanAbsRoll(mRoll);
          setLastMeanAbsPitch(mPitch);

          setHistory((h) =>
            [
              ...h,
              {
                startedAt: start,
                endedAt: ts,
                durationSec: durSec,
                meanAbsRoll: mRoll,
                meanAbsPitch: mPitch,
              },
            ].slice(-100)
          );
        } else if (startTimeRef.current) {
          // Si el vuelo fue muy corto, no lo contamos pero reseteamos
          setCurrentFlightSec(0);
        }

        // reset sesión
        setIsFlying(false);
        startTimeRef.current = null;
        prevTsRef.current = null;
        sumAbsRollRef.current = 0;
        sumAbsPitchRef.current = 0;
        countRef.current = 0;
        setCurrentFlightSec(0);
        setMeanAbsRoll(0);
        setMeanAbsPitch(0);
      }

      return;
    }

    // No vuelo → actualizar sólo ts previo
    if (!isFlying) prevTsRef.current = ts;
  }, [sample, isFlying, debouncedThrottle, THR_START, THR_END, MIN_SESSION_MS, meanAbsRoll, meanAbsPitch]);

  const resetTotals = useCallback(() => {
    setTotalFlightSec(0);
    setLastFlightSec(0);
    setLastMeanAbsRoll(0);
    setLastMeanAbsPitch(0);
    setHistory([]);
    localStorage.setItem(LS_TOTAL, "0");
    localStorage.setItem(LS_LAST, "0");
    localStorage.setItem(LS_HISTORY, "[]");
  }, []);

  return {
    // Estado actual
    isFlying,
    currentFlightSec,

    // Última sesión
    lastFlightSec,
    lastMeanAbsRoll,
    lastMeanAbsPitch,

    // Acumulado + historial
    totalFlightSec,
    history,

    // Medias en curso
    meanAbsRoll,
    meanAbsPitch,

    // Throttle (número + etiqueta + umbrales para mostrar)
    throttle: {
      value: throttleValue,
      label: throttleLabel,
      startThreshold: THR_START,
      endThreshold: THR_END,
    },

    // Utilidades
    resetTotals,
  };
};
