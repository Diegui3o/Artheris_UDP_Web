"use client";

import { useEffect, useRef } from "react";
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
  Filler,
);

interface SpectrumChartProps {
  data: any;
}

export default function SpectrumChart({ data }: SpectrumChartProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const chartRef = useRef<Chart | null>(null);

  useEffect(() => {
    if (!canvasRef.current || !data?.error_spectrum?.frequencies_hz) return;

    // Destruir chart anterior si existe
    if (chartRef.current) {
      chartRef.current.destroy();
    }

    const ctx = canvasRef.current.getContext("2d");
    if (!ctx) return;

    const freqs = data.error_spectrum.frequencies_hz;
    const mags = data.error_spectrum.magnitudes;

    // Limitar a frecuencias hasta 12.5Hz (Nyquist)
    const maxFreq = 12.5;
    const indices = freqs
      .map((f: number, i: number) => ({ f, i }))
      .filter(({ f }: { f: number }) => f <= maxFreq);
    const filteredFreqs = indices.map(({ f }: { f: number }) => f);
    const filteredMags = indices.map(({ i }: { i: number }) => mags[i]);

    chartRef.current = new Chart(ctx, {
      type: "line",
      data: {
        labels: filteredFreqs.map((f: number) => f.toFixed(1)),
        datasets: [
          {
            label: "Espectro del Error",
            data: filteredMags,
            borderColor: "rgb(34, 197, 94)",
            backgroundColor: "rgba(34, 197, 94, 0.1)",
            borderWidth: 2,
            pointRadius: 0,
            pointHoverRadius: 4,
            fill: true,
            tension: 0.2,
          },
        ],
      },
      options: {
        responsive: true,
        maintainAspectRatio: true,
        plugins: {
          legend: {
            labels: { color: "#9ca3af" },
          },
          tooltip: {
            mode: "index",
            intersect: false,
            callbacks: {
              label: (context) => {
                const value = context.parsed.y;
                return `Magnitud: ${value.toFixed(4)}`;
              },
            },
          },
        },
        scales: {
          x: {
            title: {
              display: true,
              text: "Frecuencia (Hz)",
              color: "#9ca3af",
            },
            ticks: { color: "#9ca3af" },
            grid: { color: "#374151" },
          },
          y: {
            title: {
              display: true,
              text: "Magnitud",
              color: "#9ca3af",
            },
            ticks: { color: "#9ca3af" },
            grid: { color: "#374151" },
          },
        },
      },
    });

    return () => {
      if (chartRef.current) {
        chartRef.current.destroy();
      }
    };
  }, [data]);

  if (!data?.error_spectrum?.frequencies_hz?.length) {
    return (
      <div className="text-center py-12 text-gray-400">
        <p>📊 No hay datos de espectro disponibles para este vuelo</p>
        <p className="text-sm mt-2">
          Asegúrate de que el vuelo tenga suficientes muestras
        </p>
      </div>
    );
  }

  const peaks = data.error_spectrum.dominant_peaks || [];

  return (
    <div className="space-y-6">
      <h3 className="text-lg font-semibold text-white mb-4 flex items-center gap-2">
        <span className="w-2 h-2 bg-green-500 rounded-full"></span>
        Análisis Espectral (FFT)
      </h3>

      <div className="bg-gray-900 rounded-xl p-4">
        <canvas
          ref={canvasRef}
          height={300}
          style={{ maxHeight: "300px", width: "100%" }}
        />
      </div>

      {peaks.length > 0 && (
        <div className="bg-gray-900 rounded-xl p-4">
          <h4 className="text-sm font-semibold text-gray-300 mb-3">
            🔍 Frecuencias dominantes
          </h4>
          <div className="flex flex-wrap gap-3">
            {peaks.slice(0, 5).map((peak: any, idx: number) => (
              <div key={idx} className="bg-gray-800 rounded-lg px-3 py-2">
                <div className="text-xs text-gray-400">Frecuencia</div>
                <div className="text-lg font-bold text-green-400">
                  {peak.frequency_hz.toFixed(2)} Hz
                </div>
                <div className="text-xs text-gray-500">
                  Magnitud: {peak.magnitude.toFixed(4)}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {data.correlations && data.correlations.length > 0 && (
        <div className="bg-blue-900/30 border border-blue-500/50 rounded-xl p-4">
          <h4 className="text-sm font-semibold text-blue-300 mb-2">
            🔗 Correlaciones detectadas
          </h4>
          {data.correlations.map((corr: any, idx: number) => (
            <div key={idx} className="text-sm text-gray-300">
              • {corr.description}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
