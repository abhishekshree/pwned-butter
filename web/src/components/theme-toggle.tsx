"use client";

import { useEffect, useRef, useState } from "react";
import { Moon, Sun } from "lucide-react";

import { Button } from "@/components/ui/button";

type Mode = "system" | "light" | "dark";

function systemIsDark(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches
  );
}

function applyMode(mode: Mode) {
  const dark = mode === "dark" || (mode === "system" && systemIsDark());
  document.documentElement.classList.toggle("dark", dark);
}

export function ThemeToggle() {
  const [mounted, setMounted] = useState(false);
  const [mode, setMode] = useState<Mode>("system");
  const modeRef = useRef<Mode>("system");

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect
    setMounted(true);
    const stored = (localStorage.getItem("theme") ?? "system") as Mode;
    setMode(stored);
    modeRef.current = stored;
    applyMode(stored);

    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => {
      if (modeRef.current === "system") applyMode("system");
    };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const cycle = () => {
    const next: Mode =
      mode === "system" ? (systemIsDark() ? "light" : "dark") : "system";
    setMode(next);
    modeRef.current = next;
    localStorage.setItem("theme", next);
    applyMode(next);
  };

  const dark = mounted && document.documentElement.classList.contains("dark");

  return (
    <Button
      variant="ghost"
      size="icon"
      aria-label={dark ? "Switch to light theme" : "Switch to dark or system theme"}
      onClick={cycle}
    >
      {dark ? <Sun className="size-4" /> : <Moon className="size-4" />}
    </Button>
  );
}