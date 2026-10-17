// Data Dragon images, fetched through Rust (cached on disk) and memoized here.
import { createContext, useContext, useEffect, useState } from "react";
import { api } from "../api";
import type { ChampionEntry } from "../types";

const cache = new Map<string, Promise<string>>();

function cached(key: string, load: () => Promise<string>): Promise<string> {
  let p = cache.get(key);
  if (!p) {
    p = load();
    cache.set(key, p);
    // Allow a retry later if this fails (e.g. offline).
    p.catch(() => cache.delete(key));
  }
  return p;
}

function useAsset(key: string | null, load: () => Promise<string>): string | null {
  const [src, setSrc] = useState<{ key: string; url: string } | null>(null);
  useEffect(() => {
    if (!key) return;
    let alive = true;
    cached(key, load).then(
      (url) => alive && setSrc({ key, url }),
      () => {},
    );
    return () => {
      alive = false;
    };
    // `load` is derived from `key`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  return src && src.key === key ? src.url : null;
}

export function ChampionIcon({ id, patch, size = 28 }: { id: number; patch: string; size?: number }) {
  const src = useAsset(`champ:${patch}:${id}`, () => api.championIcon(id, patch));
  const style = { width: size, height: size };
  return src ? (
    <img className="icon champ-icon" style={style} src={src} alt="" />
  ) : (
    <span className="icon placeholder" style={style} />
  );
}

export function ItemIcon({ id, patch, size = 24 }: { id: number; patch: string; size?: number }) {
  const src = useAsset(id > 0 ? `item:${patch}:${id}` : null, () => api.itemIcon(id, patch));
  const style = { width: size, height: size };
  if (id <= 0) return <span className="icon empty" style={style} />;
  return src ? (
    <img className="icon" style={style} src={src} alt="" />
  ) : (
    <span className="icon placeholder" style={style} />
  );
}

export function SpellIcon({ id, patch, size = 18 }: { id: number; patch: string; size?: number }) {
  const src = useAsset(id > 0 ? `spell:${patch}:${id}` : null, () => api.spellIcon(id, patch));
  const style = { width: size, height: size };
  return src ? (
    <img className="icon" style={style} src={src} alt="" />
  ) : (
    <span className="icon placeholder" style={style} />
  );
}

export function useSplash(champion: string | null): string | null {
  return useAsset(champion ? `splash:${champion}` : null, () => api.championSplash(champion ?? ""));
}

export function useMinimap(patch: string | null): string | null {
  return useAsset(patch ? `map:${patch}` : null, () => api.minimap(patch ?? ""));
}

/** Champion display names ("Leblanc" → "LeBlanc"), keyed by numeric id. */
export const ChampionsContext = createContext<Map<number, ChampionEntry>>(new Map());

export function useChampionName() {
  const champs = useContext(ChampionsContext);
  return (id: number, fallback: string) => champs.get(id)?.name ?? fallback;
}
