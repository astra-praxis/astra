/**
 * 手机遥控页：桌面端开启局域网桥 → 出二维码 → 手机扫码即绑定。
 *
 * 手机端（浏览器打开，无需装 App）可以看到桌面全部会话与正在进行的对话，
 * 发消息会进入 PC 的任务队列排队续发，也可以调整队列顺序、
 * 代答权限与提问卡片。本页只管桥的生命周期（开启/停止/刷新）与配对状态展示。
 */
import { useCallback, useEffect, useState } from 'react';
import { Check, Copy, Play, RefreshCw, Smartphone, Square, WifiOff } from 'lucide-react';
import { ensureMobileBridge, useMobileBridgeStore } from '@/stores/mobile';
import './RemotePage.scss';

export default function RemotePage() {
  const info = useMobileBridgeStore((s) => s.info);
  const running = useMobileBridgeStore((s) => s.running);
  const paired = useMobileBridgeStore((s) => s.paired);
  const starting = useMobileBridgeStore((s) => s.starting);
  const error = useMobileBridgeStore((s) => s.error);
  const start = useMobileBridgeStore((s) => s.start);
  const stop = useMobileBridgeStore((s) => s.stop);
  const refresh = useMobileBridgeStore((s) => s.refresh);

  const [urlCopied, setUrlCopied] = useState(false);

  useEffect(() => {
    void ensureMobileBridge();
    void refresh();
  }, [refresh]);

  // 配对数变化由 pair 事件驱动；轮询兜底对账（如手机直接关页面）
  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => void refresh(), 8000);
    return () => window.clearInterval(timer);
  }, [running, refresh]);

  const copyUrl = useCallback(async () => {
    if (!info?.url) return;
    try {
      await navigator.clipboard.writeText(info.url);
      setUrlCopied(true);
      window.setTimeout(() => setUrlCopied(false), 1500);
    } catch {
      // 剪贴板不可用时忽略
    }
  }, [info]);

  return (
    <div className="remote-page">
      <header className="remote-page__head">
        <h2>
          <Smartphone size={18} /> 手机遥控
        </h2>
        <p>
          在同一局域网内用手机扫码（或手输地址）绑定这台桌面端：手机上可查看全部会话与
          正在进行的对话，发消息会进入 PC 的任务队列排队续发，也可以调整队列顺序、
          代答权限与提问卡片。
        </p>
      </header>

      {error && (
        <div className="remote-page__error" role="alert">
          {error}
        </div>
      )}

      {!running ? (
        <section className="remote-page__card remote-page__card--empty">
          <WifiOff size={28} strokeWidth={1.6} />
          <p>桥服务未开启，手机还无法连接。</p>
          <button type="button" className="remote-page__primary" disabled={starting} onClick={() => void start()}>
            <Play size={14} />
            {starting ? '开启中…' : '开启手机遥控'}
          </button>
        </section>
      ) : (
        <>
          <section className="remote-page__card remote-page__status">
            <span className={`remote-page__dot${paired > 0 ? ' is-on' : ''}`} />
            <span className="remote-page__state">
              {paired > 0 ? `已连接 ${paired} 台手机` : '等待手机扫码…'}
            </span>
            <span className="remote-page__lan">局域网 {info?.lan_ip}</span>
            <button type="button" className="remote-page__icon-btn" onClick={() => void refresh()} title="刷新状态">
              <RefreshCw size={14} />
            </button>
            <button type="button" className="remote-page__danger" onClick={() => void stop()}>
              <Square size={12} strokeWidth={3} />
              停止
            </button>
          </section>

          <section className="remote-page__card remote-page__qr">
            <div
              className="remote-page__qr-box"
              // qrcode crate 生成的可信 SVG（内容是本页入口地址）
              dangerouslySetInnerHTML={{ __html: info?.qr_svg ?? '' }}
              aria-label="手机扫码绑定入口二维码"
            />
            <div className="remote-page__qr-side">
              <h3>手机扫码即绑定</h3>
              <ol className="remote-page__steps">
                <li>手机连接同一 Wi-Fi / 局域网</li>
                <li>相机或扫码工具识别左侧二维码</li>
                <li>在浏览器中打开，即可看到桌面会话</li>
              </ol>
              <div className="remote-page__url">
                <code className="remote-page__url-text">{info?.url}</code>
                <button type="button" className="remote-page__icon-btn" onClick={() => void copyUrl()} title="复制地址">
                  {urlCopied ? <Check size={14} /> : <Copy size={14} />}
                </button>
              </div>
              <p className="remote-page__hint">
                二维码等同钥匙：同一局域网内拿到链接的设备都能接入。停止服务或重启后旧二维码失效，
                需重新扫码。链接只在局域网内有效，不会暴露到公网。
              </p>
            </div>
          </section>
        </>
      )}
    </div>
  );
}
