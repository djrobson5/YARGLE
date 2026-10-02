// Which songs a toolbar action (Decrypt, Rename, …) covers: the checked songs,
// the songs a search/origin filter leaves visible, or the whole library.
export type TargetScope = "selected" | "shown" | "all";

// "2 selected songs", "all 37 shown songs", "all 21,151 songs".
export function describeTargets(count: number, scope: TargetScope, noun = "song"): string {
  const n = count.toLocaleString();
  const nouns = count === 1 ? noun : `${noun}s`;
  if (scope === "selected") return `${n} selected ${nouns}`;
  if (scope === "shown") return count === 1 ? `the 1 shown ${noun}` : `all ${n} shown ${nouns}`;
  return count === 1 ? `the 1 ${noun}` : `all ${n} ${nouns}`;
}
