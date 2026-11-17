import { createSignal, onMount, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import type { MediaBackendInfo } from "@go-player/contracts";
import "./App.css";

function App() {
  const [backend, setBackend] = createSignal<MediaBackendInfo | null>(null);

  onMount(async () => {
    setBackend(await invoke<MediaBackendInfo>("media_backend"));
  });

  return (
    <main class="flex h-full flex-col bg-zinc-950 text-zinc-100">
      <header class="flex h-11 shrink-0 items-center border-b border-zinc-800 px-4">
        <h1 class="text-sm font-semibold tracking-wide">Go Player</h1>
        <span class="ml-auto text-xs text-zinc-500">
          playback engine:{" "}
          <span class="text-zinc-300">{backend()?.name ?? "…"}</span>
        </span>
      </header>

      <section class="relative flex flex-1 items-center justify-center bg-black">
        <div class="text-center text-zinc-600">
          <p class="text-lg">No media loaded</p>
          <p class="mt-1 text-sm">
            Video surface lands here once the libmpv backend is wired up.
          </p>
        </div>
        {/* Subtitle overlay layer will render above the native video surface. */}
        <div class="pointer-events-none absolute inset-x-0 bottom-16 flex justify-center px-8">
          <p class="text-2xl font-medium text-white drop-shadow-[0_1px_2px_rgba(0,0,0,0.9)]">
            Subtitle preview goes here
          </p>
        </div>
      </section>

      <footer class="flex h-14 shrink-0 items-center gap-3 border-t border-zinc-800 px-4">
        <button class="rounded bg-zinc-800 px-3 py-1.5 text-sm hover:bg-zinc-700 disabled:opacity-40" disabled>
          Play
        </button>
        <input
          type="range"
          min="0"
          max="100"
          value="0"
          class="h-1 flex-1 accent-zinc-100"
          disabled
          aria-label="Seek"
        />
        <span class="w-24 text-right text-xs tabular-nums text-zinc-500">
          00:00 / 00:00
        </span>
      </footer>

      <Show when={false}>{/* settings / track picker panels mount here */}</Show>
    </main>
  );
}

export default App;
