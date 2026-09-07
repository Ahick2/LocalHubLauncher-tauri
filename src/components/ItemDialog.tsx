import { useState, type FormEvent } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { desktop, errorMessage } from '../bridge';
import { launchTypes, type LaunchItem } from '../types';
import { Icon } from './Icon';
import { Modal, Switch } from './Modal';

export function ItemDialog({ item, editing, onSave, onClose }: {
  item: LaunchItem; editing: boolean; onSave: (item: LaunchItem) => Promise<void>; onClose: () => void;
}) {
  const [draft, setDraft] = useState({ ...item });
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const change = <K extends keyof LaunchItem>(key: K, value: LaunchItem[K]) => setDraft(previous => ({ ...previous, [key]: value }));
  const hint = launchTypes.find(type => type.value === draft.launchType)?.hint;

  async function browse(directory = false) {
    try {
      const path = await open({ directory, multiple: false, title: directory ? '选择工作目录' : '选择启动文件',
        ...(!directory ? { filters: [{ name: '程序和脚本', extensions: ['exe', 'com', 'bat', 'cmd', 'ps1', 'py', 'pyw'] }, { name: '所有文件', extensions: ['*'] }] } : {}) });
      if (typeof path !== 'string') return;
      if (directory) change('workingDirectory', path);
      else {
        const fileName = path.split(/[\\/]/).at(-1) ?? '';
        setDraft(previous => ({ ...previous, target: path, launchType: 'Auto',
          name: previous.name || fileName.replace(/\.[^.]+$/, ''),
          workingDirectory: previous.workingDirectory || path.slice(0, Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))) }));
      }
    } catch (reason) { setError(errorMessage(reason)); }
  }

  async function submit(event: FormEvent) {
    event.preventDefault();
    setError('');
    if (!draft.name.trim() || !draft.target.trim()) { setError('请填写名称和启动文件或命令。'); return; }
    setBusy(true);
    try { await onSave({ ...draft, name: draft.name.trim(), target: draft.target.trim(),
      category: draft.category.trim() || '本地服务', workingDirectory: draft.workingDirectory.trim(), url: draft.url.trim() }); }
    catch (reason) { setError(errorMessage(reason)); }
    finally { setBusy(false); }
  }

  return <Modal title={editing ? '编辑启动项' : '添加启动项'} subtitle="把常用的程序与服务放在一起，随时启动。" onClose={onClose} busy={busy}>
    <form onSubmit={event => void submit(event)}>
      <div className="modal-body">
        <fieldset disabled={busy} className="form-fields">
          <div className="two-columns">
            <label className="field">名称 <span className="required">*</span>
              <input autoFocus maxLength={200} required value={draft.name} placeholder="例如：开发 API" onChange={event => change('name', event.target.value)} /></label>
            <label className="field">分类
              <input maxLength={200} value={draft.category} placeholder="本地服务" onChange={event => change('category', event.target.value)} /></label>
          </div>
          <label className="field">启动方式
            <select value={draft.launchType} onChange={event => change('launchType', event.target.value as LaunchItem['launchType'])}>
              {launchTypes.map(type => <option value={type.value} key={type.value}>{type.label}</option>)}
            </select><span className="field-hint">{hint}</span>
          </label>
          <label className="field">启动文件或命令 <span className="required">*</span>
            <div className="input-with-button"><textarea rows={2} required spellCheck={false} value={draft.target}
              placeholder="选择文件，或输入 node server.js" onChange={event => change('target', event.target.value)} />
              <button className="button browse-button" type="button" disabled={!desktop} onClick={() => void browse()} title="选择启动文件"><Icon name="folder" /></button></div>
          </label>
          <label className="field">启动参数
            <input spellCheck={false} value={draft.arguments} placeholder='例如：--host 0.0.0.0 --port 8000'
              onChange={event => change('arguments', event.target.value)} /></label>
          <label className="field">工作目录
            <div className="input-with-button"><input spellCheck={false} value={draft.workingDirectory} placeholder="留空使用文件所在目录或启动器目录"
              onChange={event => change('workingDirectory', event.target.value)} />
              <button className="button browse-button" type="button" disabled={!desktop} onClick={() => void browse(true)} title="选择工作目录"><Icon name="folder" /></button></div>
          </label>
          <label className="field">服务网址
            <input spellCheck={false} value={draft.url} placeholder="http://localhost:8000" onChange={event => change('url', event.target.value)} /></label>
          <div className="item-options">
            <label><input type="checkbox" checked={draft.hideWindow} onChange={event => change('hideWindow', event.target.checked)} />后台运行并捕获日志</label>
            <label><input type="checkbox" checked={draft.autoStart} onChange={event => change('autoStart', event.target.checked)} />打开启动器后自动运行</label>
            <div className="enable-option"><span>启用此项</span><Switch label="启用此项" checked={draft.enabled} onChange={value => change('enabled', value)} disabled={busy} /></div>
          </div>
        </fieldset>
        {error && <p className="form-error" role="alert"><Icon name="warning" />{error}</p>}
      </div>
      <footer className="modal-footer"><button type="button" className="button" onClick={onClose} disabled={busy}>取消</button>
        <button className="button primary" type="submit" disabled={busy || !desktop}>{busy ? '正在保存…' : '保存启动项'}</button></footer>
    </form>
  </Modal>;
}
