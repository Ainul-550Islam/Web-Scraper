// NoteRequest — split from types.ts; barrel re-exports keep every public path & name


export interface NoteRequest {
  text: string; // server caps body at 4096 bytes (MAX_BODY)
}
