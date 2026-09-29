// P — split from Icons.tsx; barrel re-exports keep every public path & name

import type { SVGProps } from "react";

export type P = SVGProps<SVGSVGElement> & { size?: number };
