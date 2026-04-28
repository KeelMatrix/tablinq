export const controlCandidateSelector = [
  "button",
  "[role='button']",
  "[role='switch']",
  "[role='tab']",
  "[role='option']",
  "[role='menuitem']",
  "[role='menuitemradio']",
  "[aria-haspopup]",
  "label",
  "a",
].join(", ");

export function orderCapabilityStrategies({
  capability,
  strategies,
  selectorPack,
  preferredStrategies,
}) {
  const fromPack =
    selectorPack?.capabilities?.[capability]
      ?.map((entry) => {
        const match = strategies.find(
          (strategy) => strategy.id === entry.strategy_id,
        );
        return match ? { ...match, score: entry.score } : null;
      })
      .filter(Boolean) ?? [];
  const orderedStrategies =
    fromPack.length > 0
      ? [
          ...fromPack,
          ...strategies.filter(
            (strategy) => !fromPack.some((entry) => entry.id === strategy.id),
          ),
        ]
      : [...strategies];
  const preferred = preferredStrategies[capability];
  if (!preferred) {
    return orderedStrategies.sort((left, right) => right.score - left.score);
  }
  return orderedStrategies.sort((left, right) => {
    if (left.id === preferred) {
      return -1;
    }
    if (right.id === preferred) {
      return 1;
    }
    return right.score - left.score;
  });
}
