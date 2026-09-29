// IconButton — split from ui.tsx; barrel re-exports keep every public path & name



export function IconButton({
  label,
  children,
  className = "",
  ...rest
}: { label: string } & React.ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={`iconbtn ${className}`}
      aria-label={label}
      data-tip={label}
      {...rest}
    >
      {children}
    </button>
  );
}
