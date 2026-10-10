"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import L from "leaflet";
import "leaflet/dist/leaflet.css";

import { buildHref } from "@/lib/href";
import type { Filters } from "@/lib/types";
import type { MapPoint } from "./area-map";

export function LeafletMap({
  points,
  filters,
}: {
  points: MapPoint[];
  filters: Filters;
}) {
  const router = useRouter();
  const ref = useRef<HTMLDivElement>(null);
  const mapRef = useRef<L.Map | null>(null);
  const [status, setStatus] = useState<"loading" | "ready" | "degraded" | "failed">("loading");

  useEffect(() => {
    const t = setTimeout(() => {
      const el = ref.current;
      if (!el) {
        setStatus("failed");
        return;
      }
      try {
        const map = L.map(el, { scrollWheelZoom: false }).setView(
          [19.07, 72.87],
          11,
        );
        mapRef.current = map;
        const tiles = L.tileLayer(
          "https://tile.openstreetmap.de/{z}/{x}/{y}.png",
          {
            attribution:
              '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
          },
        );
        tiles.on("tileerror", () => setStatus("degraded"));
        tiles.addTo(map);
        const max = points.length
          ? Math.max(...points.map((pt) => pt.n))
          : 0;
        for (const pt of points) {
          L.circleMarker([pt.lat, pt.lng], {
            radius: 6 + (max > 0 ? Math.sqrt(pt.n / max) * 10 : 0),
            color: "#ffffff",
            weight: 1.5,
            fillColor: "#f97316",
            fillOpacity: 0.75,
          })
            .bindTooltip(`${pt.key}: ${pt.n}`, {
              direction: "top",
              offset: [0, -8],
            })
            .on("click", () =>
              router.push(buildHref({ ...filters, q: pt.key })),
            )
            .addTo(map);
        }
        map.whenReady(() =>
          setStatus((s) => (s === "loading" ? "ready" : s)),
        );
      } catch {
        setStatus("failed");
      }
    }, 0);
    return () => {
      clearTimeout(t);
      mapRef.current?.remove();
      mapRef.current = null;
    };
  }, [points, filters, router]);

  if (status === "failed") {
    return (
      <div className="rounded-lg border border-border/60 bg-muted/30 p-3">
        <p className="font-mono text-[11px] text-muted-foreground">
          Map failed to start — areas still filter below:
        </p>
        <div className="mt-2 flex flex-wrap gap-1.5">
          {points.map((pt) => (
            <Link
              key={pt.key}
              href={buildHref({ ...filters, q: pt.key })}
              className="rounded-md border border-border bg-background px-2 py-1 font-mono text-[11px] text-foreground hover:bg-muted"
            >
              {pt.key} · {pt.n}
            </Link>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div>
      <div
        ref={ref}
        style={{ height: 320 }}
        className="z-0 h-80 w-full rounded-lg bg-muted/40"
      />
      <p className="mt-1 font-mono text-[10px] text-muted-foreground">
        {status === "ready"
          ? `Map live · ${points.length} areas plotted`
          : status === "degraded"
            ? "Map tiles blocked by network — dots still filter the feed"
            : "Starting map…"}
      </p>
    </div>
  );
}
