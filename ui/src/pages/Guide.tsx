// The in-app guide renders the repository README, so the app and GitHub
// always show the same document.
import { openUrl } from "@tauri-apps/plugin-opener";
import { marked } from "marked";
import { useMemo } from "react";
import readme from "../../../README.md?raw";

/** GitHub's heading anchor: "Riot's rules & X" → "riots-rules--x". */
function slug(text: string): string {
  return text
    .trim()
    .toLowerCase()
    .replace(/[^\w\- ]/g, "")
    .replace(/ /g, "-");
}

export function Guide() {
  // Our own bundled file, not user input.
  const html = useMemo(() => {
    const doc = new DOMParser().parseFromString(marked.parse(readme, { async: false }), "text/html");
    doc.querySelectorAll("h1, h2, h3").forEach((h) => (h.id = slug(h.textContent ?? "")));
    return doc.body.innerHTML;
  }, []);

  // In-page anchors scroll; web links open in the browser instead of
  // replacing the app window.
  const onClick = (e: React.MouseEvent) => {
    const a = (e.target as HTMLElement).closest("a");
    const href = a?.getAttribute("href");
    if (!href) return;
    e.preventDefault();
    if (href.startsWith("#")) {
      document.getElementById(href.slice(1))?.scrollIntoView({ behavior: "smooth", block: "start" });
    } else if (href.startsWith("https://")) {
      void openUrl(href);
    }
  };

  return (
    <div className="page">
      <section className="panel guide" onClick={onClick} dangerouslySetInnerHTML={{ __html: html }} />
    </div>
  );
}
