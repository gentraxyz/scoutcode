import { deltaTabs, getTabInfo, useDeltaTabs } from './tabs';
import { Icon } from './ui';
import { DeltaMark } from './Delta';

export function DeltaTabsBar() {
  const tabs = useDeltaTabs((s) => s.tabs);
  const activeId = useDeltaTabs((s) => s.activeId);
  const doc = useDeltaTabs((s) => s.doc);
  const workspaceKey = useDeltaTabs((s) => s.workspaceKey);

  if (!workspaceKey || !tabs.length) {
    return (
      <div className="delta-top-brand" data-tauri-drag-region>
        <DeltaMark size={16} />
        <b className="chat-title ellipsis" data-tauri-drag-region>Delta</b>
      </div>
    );
  }

  return (
    <div className="delta-tabs-container">
      <div className="delta-top-brand" data-tauri-drag-region>
        <DeltaMark size={16} />
      </div>
      <div className="delta-tabs-bar" role="tablist" aria-label="Delta tabs">
        {tabs.map((tab, idx) => {
          const info = getTabInfo(tab, doc);
          const active = tab.id === activeId;
          return (
            <div
              key={tab.id}
              role="tab"
              aria-selected={active}
              className={`delta-tab ${active ? 'active' : ''}`}
              onClick={() => deltaTabs.switchTab(tab.id)}
              onMouseDown={(e) => {
                if (e.button === 1) {
                  e.preventDefault();
                  e.stopPropagation();
                  deltaTabs.closeTab(tab.id);
                }
              }}
              title={`${info.title} (Ctrl+${idx + 1})`}
            >
              <span className="delta-tab-icon"><Icon name={info.icon} size={13} /></span>
              <span className="delta-tab-title">{info.title}</span>
              {tabs.length > 1 && (
                <button
                  type="button"
                  className="delta-tab-close"
                  aria-label={`Close ${info.title}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    deltaTabs.closeTab(tab.id);
                  }}
                >
                  <Icon name="close" size={11} />
                </button>
              )}
            </div>
          );
        })}
      </div>
      <button
        type="button"
        className="delta-tab-add"
        aria-label="New tab"
        title="New tab (Ctrl+T)"
        onClick={() => deltaTabs.addTab()}
      >
        <Icon name="plus" size={13} />
      </button>
    </div>
  );
}
