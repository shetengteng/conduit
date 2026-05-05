/**
 * Conduit landing page client logic.
 * Handles language toggle, theme toggle, screenshot lightbox, copy-to-clipboard,
 * and runtime injection of latest GitHub release assets into download cards.
 */
(function () {
  const root = document.documentElement;
  const langBtn = document.getElementById('lang-toggle');
  const langLabel = document.getElementById('lang-label');
  const themeBtn = document.getElementById('theme-toggle');

  const STORAGE_LANG = 'conduit.lang';
  const STORAGE_THEME = 'conduit.theme';

  // ---------- language ----------
  function applyLang(lang) {
    root.setAttribute('lang', lang);
    langLabel.textContent = lang === 'zh-CN' ? 'EN' : '中文';
    try {
      localStorage.setItem(STORAGE_LANG, lang);
    } catch (e) {}

    // Screenshots: 中英文版本切换。data-src 同步以让 lightbox 放大对的语言。
    const swapShot = (btnId, imgId) => {
      const btn = document.getElementById(btnId);
      const img = document.getElementById(imgId);
      if (!btn || !img) return;
      const src = lang === 'zh-CN' ? btn.dataset.srcZh : btn.dataset.srcEn;
      if (src) {
        img.src = src;
        btn.dataset.src = src;
      }
    };
    swapShot('hero-screenshot', 'hero-screenshot-img');
    swapShot('client-screenshot', 'client-screenshot-img');
  }

  const savedLang = (() => {
    try {
      return localStorage.getItem(STORAGE_LANG);
    } catch (e) {
      return null;
    }
  })();
  const initialLang =
    savedLang ||
    (navigator.language && navigator.language.toLowerCase().startsWith('zh')
      ? 'zh-CN'
      : 'en');
  applyLang(initialLang);

  langBtn.addEventListener('click', () => {
    applyLang(root.getAttribute('lang') === 'zh-CN' ? 'en' : 'zh-CN');
  });

  // ---------- theme ----------
  function applyTheme(t) {
    root.classList.toggle('dark', t === 'dark');
    try {
      localStorage.setItem(STORAGE_THEME, t);
    } catch (e) {}
  }
  const savedTheme = (() => {
    try {
      return localStorage.getItem(STORAGE_THEME);
    } catch (e) {
      return null;
    }
  })();
  const prefersDark =
    window.matchMedia &&
    window.matchMedia('(prefers-color-scheme: dark)').matches;
  applyTheme(savedTheme || (prefersDark ? 'dark' : 'light'));

  themeBtn.addEventListener('click', () => {
    applyTheme(root.classList.contains('dark') ? 'light' : 'dark');
  });

  // ---------- screenshot lightbox ----------
  (function setupLightbox() {
    const triggers = document.querySelectorAll('[data-lightbox]');
    if (!triggers.length) return;

    const overlay = document.createElement('div');
    overlay.className =
      'fixed inset-0 z-50 hidden items-center justify-center bg-black/80 p-6 backdrop-blur-sm';
    overlay.setAttribute('role', 'dialog');
    overlay.setAttribute('aria-modal', 'true');
    overlay.innerHTML = `
      <button type="button" data-close
        class="absolute right-4 top-4 inline-grid h-9 w-9 place-items-center rounded-md border border-white/20 bg-black/40 text-white transition-colors hover:bg-black/60 focus-ring"
        aria-label="Close">
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor"
             stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M18 6L6 18M6 6l12 12" />
        </svg>
      </button>
      <figure class="flex max-h-full max-w-6xl flex-col items-center gap-3">
        <img data-lightbox-img alt="" class="max-h-[80vh] w-auto rounded-lg shadow-2xl" />
        <figcaption data-lightbox-caption class="font-mono text-xs text-white/70"></figcaption>
      </figure>
    `;
    document.body.appendChild(overlay);

    const imgEl = overlay.querySelector('[data-lightbox-img]');
    const captionEl = overlay.querySelector('[data-lightbox-caption]');

    function open(src, captionZh, captionEn) {
      imgEl.src = src;
      imgEl.alt = (root.getAttribute('lang') === 'zh-CN' ? captionZh : captionEn) || '';
      captionEl.textContent =
        root.getAttribute('lang') === 'zh-CN' ? captionZh : captionEn;
      overlay.classList.remove('hidden');
      overlay.classList.add('flex');
      document.body.style.overflow = 'hidden';
    }
    function close() {
      overlay.classList.add('hidden');
      overlay.classList.remove('flex');
      imgEl.src = '';
      document.body.style.overflow = '';
    }

    triggers.forEach((t) => {
      t.addEventListener('click', () => {
        open(t.dataset.src, t.dataset.captionZh || '', t.dataset.captionEn || '');
      });
    });

    overlay.addEventListener('click', (e) => {
      if (e.target === overlay || e.target.closest('[data-close]')) close();
    });
    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && !overlay.classList.contains('hidden')) close();
    });
  })();

  // ---------- copy buttons ----------
  document.querySelectorAll('.copy-btn').forEach((btn) => {
    btn.addEventListener('click', async () => {
      const text = btn.getAttribute('data-copy') || '';
      const label = btn.querySelector('.copy-label');
      try {
        await navigator.clipboard.writeText(text);
        const prev = label.textContent;
        label.textContent = root.getAttribute('lang') === 'zh-CN' ? '已复制' : 'copied';
        btn.classList.add('text-foreground');
        setTimeout(() => {
          label.textContent = prev;
          btn.classList.remove('text-foreground');
        }, 1400);
      } catch (e) {
        label.textContent = '✗';
      }
    });
  });

  // ---------- latest release ----------
  // 走 GitHub API 探测最新 tag, 把 4 个 .dmg 直链回填到下载按钮; 失败时静默
  // 保留原有 releases/latest 跳转, 用户路径不退化。
  (function setupLatestRelease() {
    const RELEASES_API =
      'https://api.github.com/repos/shetengteng/conduit/releases/latest';
    const cards = document.querySelectorAll('[data-dmg-card]');
    if (!cards.length) return;

    const status = document.querySelectorAll('[data-dmg-status]');
    const fmtSize = (bytes) => {
      if (!bytes || bytes < 0) return '';
      const mb = bytes / (1024 * 1024);
      return mb >= 100 ? Math.round(mb) + ' MB' : mb.toFixed(1) + ' MB';
    };
    const setStatus = (zh, en) => {
      // [data-dmg-status] 在 zh / en 两个 span 内各放一份, 自己塞自己语言的内容
      status.forEach((el) => {
        const owner = el.closest('[data-i18n-zh],[data-i18n-en]');
        if (!owner) return;
        el.textContent = owner.hasAttribute('data-i18n-zh') ? zh : en;
      });
    };

    fetch(RELEASES_API, { headers: { Accept: 'application/vnd.github+json' } })
      .then((r) => {
        if (!r.ok) throw new Error('http ' + r.status);
        return r.json();
      })
      .then((release) => {
        const tag = (release && release.tag_name) || '';
        const version = tag.replace(/^v/, '');
        if (!version) throw new Error('no tag');

        // 版本号 chip
        document.querySelectorAll('[data-dmg-version]').forEach((el) => {
          el.textContent = tag;
        });

        // 资产索引: prefix + arch -> {url, size}
        const assets = (release.assets || []).reduce((acc, a) => {
          const m = /^(Conduit\.(?:Server|Client))_.*_(aarch64|x64)\.dmg$/.exec(
            a.name,
          );
          if (m) acc[m[1] + '|' + m[2]] = { url: a.browser_download_url, size: a.size };
          return acc;
        }, {});

        let allFound = true;
        cards.forEach((card) => {
          ['[data-dmg-primary]', '[data-dmg-secondary]'].forEach((sel) => {
            const link = card.querySelector(sel);
            if (!link) return;
            const key = link.dataset.assetPrefix + '|' + link.dataset.assetArch;
            const asset = assets[key];
            if (asset) {
              link.href = asset.url;
              if (link.hasAttribute('data-dmg-primary')) {
                const sizeEl = card.querySelector('[data-dmg-size]');
                if (sizeEl) sizeEl.textContent = fmtSize(asset.size);
              }
            } else {
              allFound = false;
            }
          });
        });

        const releaseDate = release.published_at
          ? new Date(release.published_at).toISOString().slice(0, 10)
          : '';
        if (allFound) {
          setStatus(
            releaseDate
              ? `最新版本 ${tag},发布于 ${releaseDate}。`
              : `最新版本 ${tag}。`,
            releaseDate ? `Latest ${tag}, released ${releaseDate}.` : `Latest ${tag}.`,
          );
        } else {
          setStatus(
            `检测到 ${tag},部分架构资产缺失,链接已回退到 Releases 页。`,
            `Detected ${tag}, but some arch assets are missing — falling back to the Releases page.`,
          );
        }
      })
      .catch(() => {
        // 静默 - 链接已是 releases/latest, 用户体验不会比改之前差。
        setStatus(
          '版本检测失败(可能 GitHub API 限流),请直接打开 Releases 页选择对应架构。',
          "Couldn't reach GitHub API — open the Releases page to pick your architecture.",
        );
      });
  })();
})();
