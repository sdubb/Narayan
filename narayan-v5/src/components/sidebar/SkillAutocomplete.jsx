import { useState, useEffect, useRef } from 'react';
import clsx from 'clsx';
import { Zap } from 'lucide-react';

function fuzzyMatch(query, text) {
  if (!query || !text) return false;
  return text.toLowerCase().includes(query.toLowerCase());
}

export default function SkillAutocomplete({ value, skills = [], onSelect }) {
  const [matches, setMatches] = useState([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [show, setShow] = useState(false);
  const ref = useRef(null);

  useEffect(() => {
    if (!value?.trim() || skills.length === 0) { setMatches([]); setShow(false); return; }
    const q = value.trim();
    const m = skills.filter(s =>
      fuzzyMatch(q, s.name) || fuzzyMatch(q, s.description) || s.aliases?.some(a => fuzzyMatch(q, a))
    ).slice(0, 5);
    setMatches(m);
    setShow(m.length > 0);
    setActiveIndex(0);
  }, [value, skills]);

  useEffect(() => {
    function handleKey(e) {
      if (!show) return;
      if (e.key === 'ArrowDown') { e.preventDefault(); setActiveIndex(i => Math.min(i + 1, matches.length - 1)); }
      if (e.key === 'ArrowUp') { e.preventDefault(); setActiveIndex(i => Math.max(i - 1, 0)); }
      if (e.key === 'Enter' && matches[activeIndex]) { e.preventDefault(); onSelect(matches[activeIndex].name); setShow(false); }
      if (e.key === 'Escape') setShow(false);
    }
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [show, matches, activeIndex, onSelect]);

  if (!show || matches.length === 0) return null;

  return (
    <div ref={ref}
      className="absolute left-0 right-0 top-full mt-1 bg-bg-card border border-border rounded-lg shadow-md overflow-hidden z-50">
      {matches.map((skill, i) => (
        <button
          key={skill.name}
          onClick={() => { onSelect(skill.name); setShow(false); }}
          className={clsx(
            'w-full flex items-center gap-2.5 px-3 py-2.5 text-left transition-colors',
            i === activeIndex ? 'bg-bg-hover' : 'hover:bg-bg-hover',
          )}
        >
          <Zap size={12} className="text-accent shrink-0" />
          <div className="flex-1 min-w-0">
            <p className="text-xs font-medium text-tx-1 truncate">{skill.name}</p>
            {skill.description && <p className="text-[10px] text-tx-3 truncate">{skill.description}</p>}
          </div>
          {skill.step_count && (
            <span className="text-[10px] font-mono text-tx-4 shrink-0">{skill.step_count} steps</span>
          )}
        </button>
      ))}
    </div>
  );
}
