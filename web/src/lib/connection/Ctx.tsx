// Ctx — split from connection.tsx; barrel re-exports keep every public path & name

import { createContext } from "react";
import type { ConnectionState } from "./ConnectionState";


export const Ctx = createContext<ConnectionState | null>(null);
