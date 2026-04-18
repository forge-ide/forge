import { type Component, createSignal, Show } from 'solid-js';
import './WhitelistedPill.css';

export interface WhitelistedPillProps {
  label: string;
  onRevoke: () => void;
}

export const WhitelistedPill: Component<WhitelistedPillProps> = (props) => {
  const [popoverOpen, setPopoverOpen] = createSignal(false);

  return (
    <div class="whitelisted-pill-wrapper">
      <button
        type="button"
        class="whitelisted-pill"
        data-testid="whitelisted-pill"
        aria-haspopup="true"
        aria-expanded={popoverOpen()}
        onClick={() => setPopoverOpen((v) => !v)}
      >
        <span class="whitelisted-pill__dot" aria-hidden="true" />
        whitelisted · {props.label}
      </button>

      <Show when={popoverOpen()}>
        <div class="whitelisted-pill__popover" data-testid="whitelist-popover" role="dialog">
          <button
            type="button"
            class="whitelisted-pill__revoke-btn"
            data-testid="revoke-btn"
            onClick={() => {
              setPopoverOpen(false);
              props.onRevoke();
            }}
          >
            Revoke for this session
          </button>
        </div>
      </Show>
    </div>
  );
};
