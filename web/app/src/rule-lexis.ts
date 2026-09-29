export const QUOTE = '"'
export const COMMENT = '#'
export const NEWLINE = '\n'

const MASK = ' '

export interface StringSpan {
  open: number
  close: number | null
}

export interface LineLexis {
  strings: StringSpan[]
  comment: number | null
}

export function lexLine(line: string): LineLexis {
  const strings: StringSpan[] = []
  let open: number | null = null
  for (let i = 0; i < line.length; i += 1) {
    const ch = line[i]
    if (open !== null) {
      if (ch === QUOTE) {
        strings.push({ open, close: i })
        open = null
      }
    } else if (ch === COMMENT) {
      return { strings, comment: i }
    } else if (ch === QUOTE) {
      open = i
    }
  }
  if (open !== null) strings.push({ open, close: null })
  return { strings, comment: null }
}

export function codeOf(line: string): string {
  const { strings, comment } = lexLine(line)
  let code = line.slice(0, comment ?? line.length)
  for (const { open, close } of strings) {
    const end = close ?? code.length
    code = code.slice(0, open + 1) + MASK.repeat(end - open - 1) + code.slice(end)
  }
  return code
}
