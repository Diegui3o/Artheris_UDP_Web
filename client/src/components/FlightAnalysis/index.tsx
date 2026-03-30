"use client";

import { useEffect, useState, useRef } from "react";
import { motion, AnimatePresence } from "framer-motion";
import MetricsTable from "./MetricsTable";
import UncertaintyChart from "./UncertaintyChart";
import SpectrumChart from "./SpectrumChart";
import MonteCarloHistogram from "./MonteCarloHistogram";
import ExportButton from "./ExportButton";

const API_BASE = import.meta.env.VITE_API_BASE || "http://localhost:3000";

interface FlightAnalysisProps {
  flightId: string;
  onClose?: () => void;
}

export default function FlightAnalysis({
  flightId,
  onClose,
}: FlightAnalysisProps) {
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [metrics, setMetrics] = useState<any>(null);
  const [spectrum, setSpectrum] = useState<any>(null);
  const [uncertainty, setUncertainty] = useState<any>(null);
  const [activeTab, setActiveTab] = useState<
    "metrics" | "spectrum" | "uncertainty"
  >("metrics");

  const reportRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const fetchData = async () => {
      setLoading(true);
      setError(null);

      try {
        const [metricsRes, spectrumRes, uncertaintyRes] = await Promise.all([
          fetch(`${API_BASE}/api/flights/${flightId}/metrics-full`),
          fetch(`${API_BASE}/api/flights/${flightId}/spectrum`),
          fetch(`${API_BASE}/api/flights/${flightId}/uncertainty`),
        ]);

        if (!metricsRes.ok) throw new Error("Error fetching metrics");
        if (!spectrumRes.ok) throw new Error("Error fetching spectrum");
        if (!uncertaintyRes.ok) throw new Error("Error fetching uncertainty");

        const metricsData = await metricsRes.json();
        const spectrumData = await spectrumRes.json();
        const uncertaintyData = await uncertaintyRes.json();

        setMetrics(metricsData);
        setSpectrum(spectrumData);
        setUncertainty(uncertaintyData);
      } catch (err) {
        setError(err instanceof Error ? err.message : "Unknown error");
      } finally {
        setLoading(false);
      }
    };

    if (flightId) {
      fetchData();
    }
  }, [flightId]);

  if (loading) {
    return (
      <div className="flex justify-center items-center h-64">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-green-500"></div>
        <span className="ml-3 text-gray-400">
          Cargando análisis del vuelo...
        </span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="bg-red-900/50 border border-red-500 rounded-xl p-6 text-center">
        <p className="text-red-300">❌ Error: {error}</p>
        <button
          onClick={() => window.location.reload()}
          className="mt-4 px-4 py-2 bg-red-600 hover:bg-red-700 rounded-lg"
        >
          Reintentar
        </button>
      </div>
    );
  }

  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      className="bg-gray-800 rounded-2xl shadow-xl overflow-hidden"
    >
      {/* Header */}
      <div className="bg-gray-900 px-6 py-4 border-b border-gray-700 flex justify-between items-center">
        <div>
          <h2 className="text-xl font-bold text-white">📊 Análisis de Vuelo</h2>
          <p className="text-sm text-gray-400 font-mono mt-1">{flightId}</p>
          {metrics?.flight_type && (
            <div className="mt-2">
              {metrics.flight_type === "reposo" && (
                <span className="inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium bg-blue-500/20 text-blue-300 border border-blue-500/50">
                  🔵 Reposo
                </span>
              )}
              {metrics.flight_type === "hover" && (
                <span className="inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium bg-green-500/20 text-green-300 border border-green-500/50">
                  🟢 Hover
                </span>
              )}
              {metrics.flight_type === "maniobra" && (
                <span className="inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium bg-orange-500/20 text-orange-300 border border-orange-500/50">
                  🟠 Maniobra
                </span>
              )}
              {metrics.flight_type === "desconocido" && (
                <span className="inline-flex items-center gap-1 px-2 py-1 rounded-full text-xs font-medium bg-gray-500/20 text-gray-300 border border-gray-500/50">
                  ⚪ Desconocido
                </span>
              )}
            </div>
          )}
        </div>

        <div className="flex items-center gap-2">
          {/* ⭐ Botón de exportar - pasamos reportRef */}
          <ExportButton
            targetRef={reportRef}
            filename={`flight_${flightId.slice(0, 8)}`}
          />
          {onClose && (
            <button
              onClick={onClose}
              className="text-gray-400 hover:text-white transition-colors"
            >
              ✕
            </button>
          )}
        </div>
      </div>

      {/* Tabs */}
      <div className="flex border-b border-gray-700 bg-gray-900/50">
        <button
          onClick={() => setActiveTab("metrics")}
          className={`px-6 py-3 font-medium transition-colors ${
            activeTab === "metrics"
              ? "border-b-2 border-green-500 text-green-400"
              : "text-gray-400 hover:text-gray-200"
          }`}
        >
          📈 Métricas
        </button>
        <button
          onClick={() => setActiveTab("spectrum")}
          className={`px-6 py-3 font-medium transition-colors ${
            activeTab === "spectrum"
              ? "border-b-2 border-green-500 text-green-400"
              : "text-gray-400 hover:text-gray-200"
          }`}
        >
          📊 Espectro FFT
        </button>
        <button
          onClick={() => setActiveTab("uncertainty")}
          className={`px-6 py-3 font-medium transition-colors ${
            activeTab === "uncertainty"
              ? "border-b-2 border-green-500 text-green-400"
              : "text-gray-400 hover:text-gray-200"
          }`}
        >
          🎯 Incertidumbre
        </button>
      </div>

      {/* ⭐ CONTENIDO CON REF PARA EXPORTAR - SOLO UNA VEZ */}
      <div ref={reportRef} className="bg-gray-800">
        <div className="p-6">
          <AnimatePresence mode="wait">
            {activeTab === "metrics" && (
              <motion.div
                key="metrics"
                initial={{ opacity: 0, x: -20 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: 20 }}
              >
                <MetricsTable data={metrics} />
              </motion.div>
            )}

            {activeTab === "spectrum" && (
              <motion.div
                key="spectrum"
                initial={{ opacity: 0, x: -20 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: 20 }}
              >
                <SpectrumChart data={spectrum} />
              </motion.div>
            )}

            {activeTab === "uncertainty" && (
              <motion.div
                key="uncertainty"
                initial={{ opacity: 0, x: -20 }}
                animate={{ opacity: 1, x: 0 }}
                exit={{ opacity: 0, x: 20 }}
              >
                <UncertaintyChart data={uncertainty} />
                <MonteCarloHistogram data={uncertainty?.monte_carlo} />
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </div>
    </motion.div>
  );
}
