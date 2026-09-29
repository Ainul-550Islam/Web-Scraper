// Button — split from ui.tsx; barrel re-exports keep every public path & name


export function Button({
  children,
  variant = "default",
  className = "",
  ...rest
}: {
  variant?: "default" | "primary" | "danger" | "ghost";
} & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={`btn ${variant === "default" ? "" : variant} ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}
