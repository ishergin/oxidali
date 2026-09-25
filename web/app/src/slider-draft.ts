export type SliderDraft = number | null

export function draftAfterPoll(draft: SliderDraft, polled: number | null | undefined): SliderDraft {
  return draft !== null && polled === draft ? null : draft
}

export function draftAfterRefusal(draft: SliderDraft, refused: number): SliderDraft {
  return draft === refused ? null : draft
}
