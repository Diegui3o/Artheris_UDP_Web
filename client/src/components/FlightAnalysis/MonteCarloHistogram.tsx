"use client";

import { useEffect, useRef } from "react";
import {
  Chart,
  BarElement,
  BarController,
  CategoryScale,
  LinearScale,
  Tooltip,
  Legend,
} from "chart.js";

Chart.register(
  BarElement,
  BarController,
  CategoryScale,
  LinearScale,
  Tooltip,
  Legend,
);

interface MonteCarloHistogramProps {
  data: any;
}

export default function MonteCarloHistogram({
  data,
}: MonteCarloHistogramProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const chartRef = useRef<Chart | null>(null);

  useEffect(() => {
    if (!canvasRef.current || !data?.histogram_bins?.length) return;

    if (chartRef.current) {
      chartRef.current.destroy();
    }

    const ctx = canvasRef.current.getContext("2d");
    if (!ctx) return;

    const bins = data.histogram_bins;
    const labels = bins.map(
      (bin: any) =>
        `${bin.lower_bound.toFixed(2)} a ${bin.upper_bound.toFixed(2)}`,
    );
    const counts = bins.map((bin: any) => bin.count);

    chartRef.current = new Chart(ctx, {
      type: "bar",
      data: {
        labels,
        datasets: [
          {
            label: "Frecuencia",
            data: counts,
            backgroundColor: "rgba(34, 197, 94, 0.7)",
            borderColor: "rgb(34, 197, 94)",
            borderWidth: 1,
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
            callbacks: {
              label: (context) => `Frecuencia: ${context.parsed.y}`,
            },
          },
        },
        scales: {
          x: {
            title: {
              display: true,
              text: "Error (°)",
              color: "#9ca3af",
            },
            ticks: {
              color: "#9ca3af",
              maxRotation: 45,
              minRotation: 45,
              autoSkip: true,
              maxTicksLimit: 10,
            },
            grid: { color: "#374151" },
          },
          y: {
            title: {
              display: true,
              text: "Número de iteraciones",
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

  if (!data?.histogram_bins?.length) {
    return null;
  }

  return (
    <div className="mt-6">
      <h3 className="text-lg font-semibold text-white mb-4 flex items-center gap-2">
        <span className="w-2 h-2 bg-orange-500 rounded-full"></span>
        Simulación Monte Carlo
      </h3>

      <div className="bg-gray-900 rounded-xl p-4">
        <canvas
          ref={canvasRef}
          height={250}
          style={{ maxHeight: "250px", width: "100%" }}
        />
      </div>

      <div className="grid grid-cols-2 md:grid-cols-5 gap-3 mt-4">
        <div className="bg-gray-900 rounded-lg p-3 text-center">
          <div className="text-xs text-gray-400">Iteraciones</div>
          <div className="text-lg font-bold text-white">
            {data.iterations?.toLocaleString()}
          </div>
        </div>
        <div className="bg-gray-900 rounded-lg p-3 text-center">
          <div className="text-xs text-gray-400">Media</div>
          <div className="text-md font-mono text-gray-300">
            {data.mean_error?.toFixed(4)}°
          </div>
        </div>
        <div className="bg-gray-900 rounded-lg p-3 text-center">
          <div className="text-xs text-gray-400">Desv. Estándar</div>
          <div className="text-md font-mono text-gray-300">
            {data.std_dev_error?.toFixed(4)}°
          </div>
        </div>
        <div className="bg-gray-900 rounded-lg p-3 text-center">
          <div className="text-xs text-gray-400">Mediana (p50)</div>
          <div className="text-md font-mono text-gray-300">
            {data.percentiles?.p50?.toFixed(4)}°
          </div>
        </div>
        <div className="bg-gray-900 rounded-lg p-3 text-center">
          <div className="text-xs text-gray-400">p95</div>
          <div className="text-md font-mono text-gray-300">
            {data.percentiles?.p95?.toFixed(4)}°
          </div>
        </div>
      </div>
    </div>
  );
}
