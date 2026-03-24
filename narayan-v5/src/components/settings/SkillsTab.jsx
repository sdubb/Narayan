import { useState, useEffect } from 'react';
import clsx from 'clsx';
import { BookOpen, Download, Upload, Loader2, Plus, CheckCircle2, X } from 'lucide-react';
import { skills } from '../../api';

export default function SkillsTab({ onFlash }) {
  const [marketplace, setMarketplace] = useState([]);
  const [installed, setInstalled] = useState([]);
  const [loading, setLoading] = useState(true);
  const [showUpload, setShowUpload] = useState(false);
  const [form, setForm] = useState({ name: '', description: '', steps: '', author: '' });
  const [uploading, setUploading] = useState(false);

  useEffect(() => {
    Promise.all([
      skills.list().catch(() => ({ skills: [] })),
      skills.registry().catch(() => ({ skills: [] })),
    ]).then(([m, r]) => { setMarketplace(m.skills || []); setInstalled(r.skills || []); })
      .finally(() => setLoading(false));
  }, []);

  async function install(name) {
    try { await skills.install(name); onFlash?.('Skill installed'); const r = await skills.registry(); setInstalled(r.skills || []); }
    catch (e) { onFlash?.(e.message); }
  }

  async function upload() {
    setUploading(true);
    try {
      const stepList = form.steps.split('\n').filter(s => s.trim()).map(s => ({ description: s.trim() }));
      await skills.upload(form.name, form.description, stepList, form.author);
      onFlash?.('Skill published');
      setShowUpload(false);
      setForm({ name: '', description: '', steps: '', author: '' });
      const m = await skills.list();
      setMarketplace(m.skills || []);
    } catch (e) { onFlash?.(e.message); }
    finally { setUploading(false); }
  }

  if (loading) return <div className="flex justify-center py-16"><Loader2 size={20} className="text-tx-4 animate-spin" /></div>;

  return (
    <div className="space-y-6">
      {/* Marketplace */}
      <div>
        <p className="text-sm font-semibold text-tx-1 mb-3">Marketplace</p>
        <div className="grid grid-cols-2 gap-3">
          {marketplace.map(s => {
            const isInstalled = installed.some(i => i.name === s.name);
            return (
              <div key={s.name} className="card p-4">
                <p className="text-sm font-medium text-tx-1">{s.name}</p>
                {s.author && <p className="text-[11px] text-tx-4">by {s.author}</p>}
                <p className="text-xs text-tx-3 mt-1 line-clamp-2">{s.description}</p>
                <div className="flex items-center justify-between mt-3">
                  <span className="text-[10px] font-mono text-tx-4">{s.steps?.length || 0} steps</span>
                  {isInstalled ? (
                    <span className="badge bg-ok-soft text-ok border border-ok/20"><CheckCircle2 size={10} /> Installed</span>
                  ) : (
                    <button onClick={() => install(s.name)} className="btn-primary text-xs px-3 py-1.5 flex items-center gap-1">
                      <Download size={11} /> Install
                    </button>
                  )}
                </div>
              </div>
            );
          })}
          {marketplace.length === 0 && <p className="text-xs text-tx-3 col-span-2">No skills in marketplace yet.</p>}
        </div>
      </div>

      {/* Installed */}
      {installed.length > 0 && (
        <div>
          <p className="text-sm font-semibold text-tx-1 mb-3">Installed</p>
          <div className="space-y-2">
            {installed.map(s => (
              <div key={s.name} className="card p-3 flex items-center gap-3">
                <BookOpen size={14} className="text-tx-3 shrink-0" />
                <span className="text-sm text-tx-1 flex-1">{s.name}</span>
                <span className="text-[10px] font-mono text-tx-4">{s.steps?.length || 0} steps</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Upload */}
      {showUpload ? (
        <div className="card p-5 space-y-3">
          <div className="flex items-center justify-between">
            <span className="text-sm font-semibold text-tx-1">Publish skill</span>
            <button onClick={() => setShowUpload(false)} className="text-tx-4 hover:text-tx-2"><X size={14} /></button>
          </div>
          <input value={form.name} onChange={e => setForm(f => ({ ...f, name: e.target.value }))} placeholder="Skill name" className="input-field text-sm" />
          <input value={form.author} onChange={e => setForm(f => ({ ...f, author: e.target.value }))} placeholder="Author" className="input-field text-sm" />
          <input value={form.description} onChange={e => setForm(f => ({ ...f, description: e.target.value }))} placeholder="Description" className="input-field text-sm" />
          <textarea value={form.steps} onChange={e => setForm(f => ({ ...f, steps: e.target.value }))} placeholder="Steps (one per line)" rows={4} className="input-field text-sm resize-none" />
          <button onClick={upload} disabled={uploading || !form.name.trim()} className="btn-primary w-full flex items-center justify-center gap-2 disabled:opacity-50">
            {uploading ? <Loader2 size={14} className="animate-spin" /> : <Upload size={14} />} Publish
          </button>
        </div>
      ) : (
        <button onClick={() => setShowUpload(true)} className="w-full rounded-xl border-2 border-dashed border-border hover:border-accent/40 p-4 flex items-center justify-center gap-2 text-sm text-tx-3 hover:text-tx-1 transition-colors">
          <Plus size={16} /> Publish a skill
        </button>
      )}
    </div>
  );
}
