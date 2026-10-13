/**
 * 星球轨道插画（空状态装饰）
 *
 * 设计：青墨绿主色星球 + 呼吸光晕 + 双轨道 + 环绕卫星 + 闪烁星点。
 * 颜色全部走主题变量（--brand-* / --accent-*），亮暗主题自动适配；
 * 动画纯 CSS（rotate / opacity 合成器友好），遵循 prefers-reduced-motion。
 * useId 生成渐变 id，同页多实例不冲突。
 */
import { useId } from 'react';
import './OrbArt.scss';

interface Props {
  /** 渲染尺寸（正方形，px），默认 128 */
  size?: number;
  className?: string;
}

export default function OrbArt({ size = 128, className = '' }: Props) {
  const uid = useId();
  // useId 返回 «r0» 形式（含特殊字符），不能直接进 url(#..)，做一次清洗
  const id = uid.replace(/[^a-zA-Z0-9_-]/g, '');
  const coreGrad = `orbart-core-${id}`;
  const glowGrad = `orbart-glow-${id}`;

  return (
    <svg
      className={`orbart ${className}`}
      width={size}
      height={size}
      viewBox="0 0 160 160"
      role="img"
      aria-label="星野"
    >
      <defs>
        {/* 星球本体：左上高光 → 品牌深色 */}
        <radialGradient id={coreGrad} cx="35%" cy="30%" r="80%">
          <stop offset="0%" style={{ stopColor: 'var(--brand-400)' }} />
          <stop offset="55%" style={{ stopColor: 'var(--brand-500)' }} />
          <stop offset="100%" style={{ stopColor: 'var(--brand-700)' }} />
        </radialGradient>
        {/* 背景光晕：accent-light → 透明 */}
        <radialGradient id={glowGrad} cx="50%" cy="50%" r="50%">
          <stop offset="0%" style={{ stopColor: 'var(--accent-light)' }} />
          <stop offset="70%" style={{ stopColor: 'var(--accent-light)' }} />
          <stop offset="100%" style={{ stopColor: 'var(--accent-light)', stopOpacity: 0 }} />
        </radialGradient>
      </defs>

      {/* 呼吸光晕 */}
      <circle className="orbart-glow" cx="80" cy="80" r="62" fill={`url(#${glowGrad})`} />

      {/* 外轨道（虚线）+ 倾斜轨道（立体感） */}
      <circle className="orbart-ring" cx="80" cy="80" r="56" />
      <ellipse
        className="orbart-ring orbart-ring--tilt"
        cx="80"
        cy="80"
        rx="68"
        ry="22"
        transform="rotate(-16 80 80)"
      />

      {/* 星球本体 + 高光 */}
      <circle className="orbart-core" cx="80" cy="80" r="30" fill={`url(#${coreGrad})`} />
      <ellipse className="orbart-highlight" cx="69" cy="68" rx="10" ry="6" transform="rotate(-24 69 68)" />

      {/* 星球表面弧线纹理（很淡，增加细节） */}
      <path
        className="orbart-detail"
        d="M 58 92 Q 80 100 102 90"
        fill="none"
      />
      <path
        className="orbart-detail"
        d="M 62 70 Q 74 62 88 66"
        fill="none"
      />

      {/* 环绕卫星（整组绕中心旋转） */}
      <g className="orbart-orbit">
        <circle className="orbart-sat" cx="80" cy="24" r="3.5" />
        <circle className="orbart-sat-glow" cx="80" cy="24" r="7" />
      </g>

      {/* 闪烁星点（4 角星，错峰） */}
      <path
        className="orbart-spark orbart-spark--1"
        d="M 24 46 Q 25.4 54.6 34 56 Q 25.4 57.4 24 66 Q 22.6 57.4 14 56 Q 22.6 54.6 24 46 Z"
      />
      <path
        className="orbart-spark orbart-spark--2"
        d="M 132 96 Q 133.2 103.2 140.5 104.5 Q 133.2 105.8 132 113 Q 130.8 105.8 123.5 104.5 Q 130.8 103.2 132 96 Z"
      />
      <path
        className="orbart-spark orbart-spark--3"
        d="M 122 30 Q 122.9 35.4 128.5 36.5 Q 122.9 37.6 122 43 Q 121.1 37.6 115.5 36.5 Q 121.1 35.4 122 30 Z"
      />
    </svg>
  );
}
