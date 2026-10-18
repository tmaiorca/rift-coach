// Ambient backdrop: the champion's splash art drifting slowly behind a dark
// veil, a turning hextech ring, and glowing gold/teal motes that drift
// upward and shy away from the cursor. Cheap by design: one canvas capped at
// ~30 fps, paused when the window is hidden or motion is reduced.
import { useEffect, useRef, useState } from "react";
import { useSplash } from "../lib/assets";

interface Props {
  /** Numeric champion key or Data Dragon id; null shows only the motes. */
  champion: string | null;
  enabled: boolean;
  particles: boolean;
  /** 0–1 */
  intensity: number;
  /** Freeze all motion (a game is running: keep the GPU and CPU for it). */
  still?: boolean;
}

export function AmbientBackground({ champion, enabled, particles, intensity, still = false }: Props) {
  const splash = useSplash(enabled ? champion : null);
  const reduced = usePrefersReducedMotion();
  const animate = enabled && !reduced && !still;
  const parallaxRef = useRef<HTMLDivElement>(null);

  // Smoothed mouse parallax on the splash layer.
  useEffect(() => {
    if (!animate) return;
    let raf = 0;
    let tx = 0;
    let ty = 0;
    let x = 0;
    let y = 0;
    const onMove = (e: MouseEvent) => {
      tx = (e.clientX / window.innerWidth - 0.5) * -18;
      ty = (e.clientY / window.innerHeight - 0.5) * -12;
    };
    const tick = () => {
      x += (tx - x) * 0.04;
      y += (ty - y) * 0.04;
      if (parallaxRef.current) parallaxRef.current.style.transform = `translate3d(${x}px, ${y}px, 0)`;
      raf = requestAnimationFrame(tick);
    };
    window.addEventListener("mousemove", onMove);
    raf = requestAnimationFrame(tick);
    return () => {
      window.removeEventListener("mousemove", onMove);
      cancelAnimationFrame(raf);
    };
  }, [animate]);

  if (!enabled) return <div className="ambient plain" aria-hidden />;
  const k = Math.min(1, Math.max(0, intensity));

  return (
    <div className="ambient" aria-hidden>
      <div className="ambient-parallax" ref={parallaxRef}>
        <SplashLayers url={splash} opacity={0.25 + 0.5 * k} animate={animate} />
      </div>
      <div className="ambient-veil" />
      <HextechRing opacity={0.05 + 0.12 * k} animate={animate} />
      {particles && <Motes intensity={k} animate={animate} />}
      <div className="ambient-vignette" />
    </div>
  );
}

/** Crossfades between splash images when the champion changes. */
function SplashLayers({ url, opacity, animate }: { url: string | null; opacity: number; animate: boolean }) {
  const [layers, setLayers] = useState<{ url: string; id: number }[]>([]);
  const [prevUrl, setPrevUrl] = useState<string | null>(null);
  if (url !== prevUrl) {
    setPrevUrl(url);
    if (url) setLayers((l) => [...l.slice(-1), { url, id: (l.at(-1)?.id ?? 0) + 1 }]);
  }
  const top = layers.at(-1)?.id;
  return (
    <>
      {layers.map((l) => (
        <div
          key={l.id}
          className={`ambient-splash ${animate ? "drift" : ""}`}
          style={{ backgroundImage: `url(${l.url})`, opacity: l.id === top ? opacity : 0 }}
        />
      ))}
    </>
  );
}

function HextechRing({ opacity, animate }: { opacity: number; animate: boolean }) {
  const ticks = Array.from({ length: 48 }, (_, i) => i);
  return (
    <svg className={`ambient-ring ${animate ? "spin" : ""}`} viewBox="-100 -100 200 200" style={{ opacity }}>
      <circle r="92" className="ring-thin" />
      <circle r="84" className="ring-dash" />
      <circle r="60" className="ring-thin" />
      <polygon points="0,-70 60.6,-35 60.6,35 0,70 -60.6,35 -60.6,-35" className="ring-hex" />
      {ticks.map((i) => {
        const a = (i / ticks.length) * Math.PI * 2;
        const long = i % 4 === 0;
        const r1 = long ? 86 : 88.5;
        return (
          <line
            key={i}
            x1={Math.cos(a) * r1}
            y1={Math.sin(a) * r1}
            x2={Math.cos(a) * 92}
            y2={Math.sin(a) * 92}
            className="ring-tick"
          />
        );
      })}
    </svg>
  );
}

interface Mote {
  x: number;
  y: number;
  vx: number;
  vy: number;
  r: number;
  hue: "gold" | "teal";
  phase: number;
}

const COLORS = { gold: [214, 172, 82], teal: [64, 200, 196] } as const;

function Motes({ intensity, animate }: { intensity: number; animate: boolean }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    let w = 0;
    let h = 0;
    const dpr = Math.min(window.devicePixelRatio || 1, 1.5);
    const resize = () => {
      w = canvas.clientWidth;
      h = canvas.clientHeight;
      canvas.width = Math.round(w * dpr);
      canvas.height = Math.round(h * dpr);
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    };
    resize();
    window.addEventListener("resize", resize);

    const count = Math.round(30 + 50 * intensity);
    const spawn = (anywhere: boolean): Mote => ({
      x: Math.random() * w,
      y: anywhere ? Math.random() * h : h + 10,
      vx: (Math.random() - 0.5) * 0.15,
      vy: -(0.12 + Math.random() * 0.35),
      r: 0.8 + Math.random() * 2.2,
      hue: Math.random() < 0.65 ? "gold" : "teal",
      phase: Math.random() * Math.PI * 2,
    });
    const motes = Array.from({ length: count }, () => spawn(true));
    const mouse = { x: -9999, y: -9999 };
    const onMove = (e: MouseEvent) => {
      mouse.x = e.clientX;
      mouse.y = e.clientY;
    };
    window.addEventListener("mousemove", onMove);

    let raf = 0;
    let last = 0;
    const draw = (t: number) => {
      raf = requestAnimationFrame(draw);
      if (document.hidden || t - last < 33) return; // ~30 fps
      const dt = last ? Math.min(3, (t - last) / 16.7) : 1;
      last = t;
      ctx.clearRect(0, 0, w, h);
      ctx.globalCompositeOperation = "lighter";
      for (const [i, m] of motes.entries()) {
        const dx = m.x - mouse.x;
        const dy = m.y - mouse.y;
        const d2 = dx * dx + dy * dy;
        let glow = 0;
        if (d2 < 140 * 140) {
          const d = Math.sqrt(d2) || 1;
          const f = (1 - d / 140) * 0.6;
          m.vx += (dx / d) * f * 0.08;
          m.vy += (dy / d) * f * 0.08;
          glow = 1 - d / 140;
        }
        m.vx *= 0.985;
        m.vy = m.vy * 0.985 - 0.004;
        m.x += m.vx * dt;
        m.y += m.vy * dt;
        m.phase += 0.03 * dt;
        if (m.y < -20 || m.x < -20 || m.x > w + 20) motes[i] = spawn(false);

        const twinkle = 0.55 + 0.45 * Math.sin(m.phase);
        const alpha = (0.18 + 0.5 * intensity) * twinkle + glow * 0.5;
        const [r, g, b] = COLORS[m.hue];
        const radius = m.r * (3.5 + glow * 3);
        const grad = ctx.createRadialGradient(m.x, m.y, 0, m.x, m.y, radius);
        grad.addColorStop(0, `rgba(${r},${g},${b},${alpha})`);
        grad.addColorStop(0.35, `rgba(${r},${g},${b},${alpha * 0.35})`);
        grad.addColorStop(1, `rgba(${r},${g},${b},0)`);
        ctx.fillStyle = grad;
        ctx.beginPath();
        ctx.arc(m.x, m.y, radius, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.globalCompositeOperation = "source-over";
    };
    if (animate) {
      raf = requestAnimationFrame(draw);
    } else {
      draw(1000);
      cancelAnimationFrame(raf);
    }
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", resize);
      window.removeEventListener("mousemove", onMove);
    };
  }, [intensity, animate]);

  return <canvas className="ambient-motes" ref={canvasRef} />;
}

function usePrefersReducedMotion(): boolean {
  const query = "(prefers-reduced-motion: reduce)";
  const [reduced, setReduced] = useState(() => window.matchMedia?.(query).matches ?? false);
  useEffect(() => {
    const mq = window.matchMedia?.(query);
    if (!mq) return;
    const on = () => setReduced(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return reduced;
}
