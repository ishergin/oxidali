export interface MembershipRow {
  desired: boolean[]
  applied: boolean[]
}

export function gearDiffers(row: MembershipRow): boolean {
  return row.desired.some((wanted, group) => wanted !== row.applied[group])
}
