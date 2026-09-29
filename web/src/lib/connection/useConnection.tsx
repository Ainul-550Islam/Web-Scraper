// useConnection — split from connection.tsx; barrel re-exports keep every public path & name

import { useContext } from "react";
import type { ConnectionState } from "./ConnectionState";
import { Ctx } from "./Ctx";


export function useConnection(): ConnectionState {
  const c = useContext(Ctx);
  if (!c) throw new Error("useConnection outside ConnectionProvider");
  return c;
}
