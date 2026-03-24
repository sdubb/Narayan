/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{js,jsx}'],
  theme: {
    extend: {
      fontFamily: {
        serif: ['Instrument Serif', 'serif'],
        sans:  ['DM Sans', 'system-ui', 'sans-serif'],
        mono:  ['DM Mono', 'monospace'],
      },
      colors: {
        bg: {
          DEFAULT: '#f5f2ee',
          card:    '#ffffff',
          hover:   '#faf8f5',
          active:  '#f0ece6',  // pressed state, progress track backgrounds
          raised:  '#fffdfb',
        },
        border: {
          DEFAULT: '#e2dbd2',
          md:      '#d1c9be',
          strong:  '#c4bab0',
        },
        tx: {
          1: '#1a1714',
          2: '#4a4540',
          3: '#8a8278',
          4: '#b0a89e',
          5: '#d0c8be',  // ultra-muted — table headers, secondary labels
        },
        accent: {
          DEFAULT: '#c96a2e',
          soft:    '#fdf0e6',
          text:    '#b35a1f',
          glow:    '#f59e0b',
        },
        ok: {
          DEFAULT: '#22c55e',
          soft:    '#f0fdf4',
        },
        err: {
          DEFAULT: '#ef4444',
          soft:    '#fef2f2',
        },
        warn: {
          DEFAULT: '#f59e0b',
          soft:    '#fffbeb',
        },
        info: {
          DEFAULT: '#3b82f6',
          soft:    '#eff6ff',
        },
        vio: {
          DEFAULT: '#8b5cf6',
          soft:    '#f5f3ff',
        },
      },
      fontSize: {
        'xs':  ['0.75rem',   { lineHeight: '1rem' }],
        'sm':  ['0.8125rem', { lineHeight: '1.25rem' }],
        'base':['0.9375rem', { lineHeight: '1.5rem' }],
        'lg':  ['1.0625rem', { lineHeight: '1.75rem' }],
        'xl':  ['1.25rem',   { lineHeight: '1.75rem' }],
        '2xl': ['1.5rem',    { lineHeight: '2rem' }],
        '3xl': ['1.875rem',  { lineHeight: '2.25rem' }],
      },
      borderRadius: {
        DEFAULT: '10px',
        lg:  '14px',
        xl:  '0.875rem',
        '2xl': '1rem',
      },
      boxShadow: {
        sm:          '0 1px 3px rgba(0,0,0,0.06), 0 1px 2px rgba(0,0,0,0.04)',
        md:          '0 4px 12px rgba(0,0,0,0.08), 0 2px 4px rgba(0,0,0,0.04)',
        card:        '0 1px 3px 0 rgba(26,23,20,0.04), 0 1px 2px -1px rgba(26,23,20,0.04)',
        'card-hover':'0 4px 12px -2px rgba(26,23,20,0.08), 0 2px 6px -2px rgba(26,23,20,0.04)',
        'card-active':'0 1px 2px 0 rgba(26,23,20,0.06)',
        'glow-amber':'0 0 20px rgba(245,158,11,0.15)',
        'glow-green':'0 0 20px rgba(34,197,94,0.15)',
        'glow-red':  '0 0 20px rgba(239,68,68,0.15)',
      },
      animation: {
        'in':        'animate-in 0.18s cubic-bezier(0.25, 0.1, 0.25, 1)',
        'fade':      'animate-fade 0.15s ease-out',
        'pulse-dot': 'pulse-dot 1.4s ease-in-out infinite',
        'spin-slow': 'spin 2.5s linear infinite',
      },
      keyframes: {
        'animate-in': {
          '0%':   { opacity: '0', transform: 'translateY(8px)' },
          '100%': { opacity: '1', transform: 'translateY(0)' },
        },
        'animate-fade': {
          '0%':   { opacity: '0' },
          '100%': { opacity: '1' },
        },
        'pulse-dot': {
          '0%, 100%': { opacity: '0.4', transform: 'scale(0.95)' },
          '50%':      { opacity: '1', transform: 'scale(1.05)' },
        },
      },
    },
  },
  plugins: [],
}
