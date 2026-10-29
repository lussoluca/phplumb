import clsx from 'clsx'

export function LogoMark(props: React.ComponentPropsWithoutRef<'svg'>) {
  return (
    <svg aria-hidden="true" viewBox="0 0 128 128" {...props}>
      <rect x="4" y="4" width="120" height="120" rx="26" fill="#3D4185" />
      <g transform="translate(11 14) scale(0.78)">
        <path
          d="M44 114 V40 Q44 26 58 26 H74 Q98 26 98 50 Q98 74 74 74 H44"
          fill="none"
          stroke="#F6F5F0"
          strokeWidth="18"
          strokeLinejoin="round"
        />
        <rect x="30" y="104" width="28" height="12" rx="2" fill="#D49A2A" />
        <rect x="64" y="12" width="9" height="28" rx="2" fill="#D49A2A" />
        <rect x="64" y="60" width="9" height="28" rx="2" fill="#D49A2A" />
      </g>
    </svg>
  )
}

export function Logo({
  className,
  ...props
}: React.ComponentPropsWithoutRef<'div'>) {
  return (
    <div className={clsx('flex items-center gap-x-3', className)} {...props}>
      <LogoMark className="h-full w-auto" />
      <span className="font-display text-2xl font-medium tracking-tight text-slate-900">
        PHPlumb
      </span>
    </div>
  )
}
