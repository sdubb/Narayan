export default {
  content: ['./index.html', './src/**/*.{js,jsx}'],
  theme: {
    extend: {
      fontFamily: {
        serif: ['Instrument Serif', 'serif'],
        sans:  ['DM Sans', 'sans-serif'],
        mono:  ['DM Mono', 'monospace'],
      },
      colors: {
        bg:      { DEFAULT:'#f5f2ee', card:'#ffffff', hover:'#f0ece6', active:'#e9e3da' },
        border:  { DEFAULT:'#e2dbd2', md:'#d4ccc0' },
        tx:      { 1:'#1a1714', 2:'#4a4540', 3:'#8a8278', 4:'#b0a89e' },
        accent:  { DEFAULT:'#c96a2e', soft:'#f5e6da', text:'#7a3d16' },
        ok:      { DEFAULT:'#2d7a4f', soft:'#e3f2e9' },
        err:     { DEFAULT:'#c0392b', soft:'#fde8e6' },
        warn:    { DEFAULT:'#b45309', soft:'#fef3c7' },
        info:    { DEFAULT:'#1d6fa4', soft:'#dbeafe' },
        vio:     { DEFAULT:'#6d28d9', soft:'#ede9fe' },
      },
      borderRadius: {
        DEFAULT: '10px',
        lg: '14px',
        xl: '18px',
      },
      boxShadow: {
        sm: '0 1px 3px rgba(0,0,0,0.06), 0 1px 2px rgba(0,0,0,0.04)',
        md: '0 4px 12px rgba(0,0,0,0.08), 0 2px 4px rgba(0,0,0,0.04)',
        card: '0 0 0 1px rgba(0,0,0,0.05), 0 2px 8px rgba(0,0,0,0.06)',
      },
    }
  },
  plugins: []
}
