// "Capture live" (doc 08): one dialog for every device. First the devices to capture from (one or
// several), then the settings: channels, rate (the same for all devices or one per device), length,
// and per device its mode, threshold and the name of its tab. Several devices capture each into its
// own tab, or all into one capture (one rate), lined up at their starts or by a signal they share.
// All are configured first and then started together.
import { useCallback, useEffect, useMemo, useState } from 'react';
import { Button, Dialog, Status, TextField } from '@logica/ui';
import { Activity, RefreshCw } from 'lucide-react';
import { hz, int } from '../format';
import { isAgent, useDeviceWatch, useStore, who } from '../store';
import { channelLabel, commonRates, configFor, defaultChoice, guessShared, problems, samplesFor, togetherProblems, togetherTitle, type Caps, type Choice, type Length } from '../captureplan';

type Device = NonNullable<ReturnType<typeof useStore>['session']>['devices'][number];

// what the person chose last time, so the dialog opens where they left it (for this session)
const remembered: {
  picked: string[];
  choices: Record<string, Choice>;
  length: Length;
  shared: boolean;
  sharedRate: number | null;
  together: boolean;
  alignOn: boolean;
  alignChannels: Record<string, number>;
} = {
  picked: [],
  choices: {},
  length: { untilStopped: false, seconds: 60 },
  shared: true,
  sharedRate: null,
  together: false,
  alignOn: true,
  alignChannels: {},
};

/** Why a device cannot be picked now, or null. */
function unavailable(d: Device): string | null {
  if (d.capturing) return 'capturing';
  if (d.holder && d.holder !== 'user') return `in use by ${who(d.holder)}`;
  return null;
}

export function CaptureDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { session, run, refresh } = useStore();
  const devices = session?.devices ?? [];
  const [step, setStep] = useState<'devices' | 'settings'>('devices');
  const [picked, setPicked] = useState<string[]>([]);
  const [choices, setChoices] = useState<Record<string, Choice>>({});
  const [length, setLength] = useState<Length>(remembered.length);
  const [shared, setShared] = useState(remembered.shared);
  const [sharedRate, setSharedRate] = useState<number | null>(remembered.sharedRate);
  // several devices: each into its own tab, or all into one capture, lined up by a shared signal or not
  const [together, setTogether] = useState(remembered.together);
  const [alignOn, setAlignOn] = useState(remembered.alignOn);
  const [alignChannels, setAlignChannels] = useState<Record<string, number>>(remembered.alignChannels);
  const [groupTitle, setGroupTitle] = useState<string | null>(null);
  const [looking, setLooking] = useState(false);
  const [starting, setStarting] = useState(false);
  // devices that appear or go while choosing show up here
  useDeviceWatch(open && step === 'devices');

  const look = useCallback(async () => {
    setLooking(true);
    const list = await run('device.list', {});
    setLooking(false);
    refresh();
    return list?.devices ?? [];
  }, [run, refresh]);

  // every time it opens: look for devices (analysers come and go) and start from the last choices
  useEffect(() => {
    if (!open) return;
    setStep('devices');
    setPicked(remembered.picked);
    setChoices(remembered.choices);
    setLength(remembered.length);
    setShared(remembered.shared);
    setSharedRate(remembered.sharedRate);
    setTogether(remembered.together);
    setAlignOn(remembered.alignOn);
    setAlignChannels(remembered.alignChannels);
    setGroupTitle(null);
    // a single free device is picked for the person
    void look().then((found) => {
      const free = found.filter((d) => unavailable(d) === null);
      if (free.length === 1 && free[0] && !remembered.picked.some((id) => free.some((d) => d.info.id === id))) setPicked([free[0].info.id]);
    });
  }, [open, look]);

  // only devices that are there and free stay picked
  const chosen = picked.filter((id) => devices.some((d) => d.info.id === id && unavailable(d) === null));

  const selected = chosen.map((id) => devices.find((d) => d.info.id === id)).filter((d): d is Device => d !== undefined);
  const rates = useMemo(() => commonRates(selected.map((d) => d.capabilities as Caps)), [selected]);
  const many = selected.length > 1;
  const asOne = many && together;
  // one capture has one rate
  const sameRate = many && (asOne || shared) && rates.length > 0;
  const rateShared = sameRate ? (rates.find((r) => r === sharedRate) ?? rates[0] ?? null) : null;

  const choiceOf = (d: Device): Choice => {
    const c = choices[d.info.id] ?? defaultChoice(d.capabilities as Caps, d.info.model);
    return rateShared !== null ? { ...c, rateHz: rateShared } : c;
  };
  const setChoice = (d: Device, p: Partial<Choice>) => setChoices((all) => ({ ...all, [d.info.id]: { ...choiceOf(d), ...p } }));

  const guess = useMemo(() => guessShared(selected.map((d) => d.capabilities as Caps)), [selected]);
  const alignList: number[] | null = asOne && alignOn ? selected.map((d, k) => alignChannels[d.info.id] ?? guess?.[k] ?? -1) : null;
  const title = groupTitle ?? togetherTitle(selected.map((d) => d.info.model));

  const plans = selected.map((d) => ({ d, caps: d.capabilities as Caps, choice: choiceOf(d) }));
  // in one capture the devices' own tab names do not apply
  const own = (p: (typeof plans)[number]) => problems(p.caps, asOne ? { ...p.choice, title } : p.choice, length, hz);
  const groupIssues = asOne
    ? [
        ...togetherProblems(plans.map((p) => ({ model: p.d.info.model, caps: p.caps, choice: p.choice })), alignList, channelLabel),
        ...(title.trim() === '' ? ['Give the capture a name.'] : []),
      ]
    : [];
  const issues = [...plans.flatMap(own), ...groupIssues];

  const start = async () => {
    setStarting(true);
    Object.assign(remembered, {
      picked: chosen,
      choices: { ...choices, ...Object.fromEntries(plans.map((p) => [p.d.info.id, p.choice])) },
      length,
      shared,
      sharedRate: rateShared ?? sharedRate,
      together,
      alignOn,
      alignChannels: { ...alignChannels, ...Object.fromEntries(selected.map((d, k) => [d.info.id, alignList?.[k] ?? alignChannels[d.info.id]]).filter(([, ch]) => ch !== undefined && ch !== -1)) },
    });
    // configure every device first, then start them one right after the other
    const ready: typeof plans = [];
    for (const p of plans) {
      const device = p.d.info.id;
      if (!p.d.connected && !(await run('device.connect', { device }))) continue;
      if (await run('device.configure', { device, config: configFor(p.caps, p.choice, length) })) ready.push(p);
    }
    if (asOne) {
      // one capture needs every device
      const [first, ...rest] = plans;
      const ok = ready.length === plans.length && first !== undefined;
      const align = alignList ? Object.fromEntries(plans.map((p, k) => [p.d.info.id, alignList[k] ?? -1])) : null;
      const r = ok
        ? await run('capture.start', { device: first.d.info.id, with: rest.map((p) => p.d.info.id), title: title.trim(), wait: false, ...(align ? { align } : {}) })
        : undefined;
      setStarting(false);
      if (r) onClose();
      return;
    }
    let started = 0;
    for (const p of ready) {
      if (await run('capture.start', { device: p.d.info.id, title: p.choice.title.trim(), wait: false })) started += 1;
    }
    setStarting(false);
    if (started > 0) onClose();
  };

  const footer =
    step === 'devices' ? (
      <>
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" disabled={selected.length === 0} onClick={() => setStep('settings')}>
          Next: settings
        </Button>
      </>
    ) : (
      <>
        <Button variant="ghost" onClick={() => setStep('devices')}>
          Back
        </Button>
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" icon={<Activity />} disabled={starting || selected.length === 0 || issues.length > 0} onClick={() => void start()}>
          {starting ? 'Starting…' : many && !asOne ? `Start ${selected.length} captures` : 'Start capture'}
        </Button>
      </>
    );

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => !o && onClose()}
      title="Capture live"
      description={
        step === 'devices'
          ? 'Choose the devices to capture from. Each captures into its own tab; they start together.'
          : 'Choose what each device captures. You can stop at any time and keep what arrived.'
      }
      wide
      footer={footer}
    >
      {step === 'devices' ? (
        <div className="stack">
          <div className="row between">
            <span className="muted">{devices.length === 0 ? 'No devices found.' : `${devices.length} found`}</span>
            <Button size="sm" variant="secondary" icon={<RefreshCw />} disabled={looking} onClick={() => void look()}>
              {looking ? 'Looking…' : 'Look again'}
            </Button>
          </div>
          {devices.length === 0 && (
            <p className="muted">
              Real analysers come with milestone M3. To try live capture now, run <code>cargo run -p logica-vla</code> in the repository (add{' '}
              <code>--listen 127.0.0.1:7172</code> for a second one), then look again.
            </p>
          )}
          <ul className="plain stack-sm" aria-label="Devices">
            {devices.map((d) => {
              const why = unavailable(d);
              const on = chosen.includes(d.info.id);
              const caps = d.capabilities;
              return (
                <li key={d.info.id} className="capture-pick">
                  <label className="check capture-pick__label">
                    <input
                      type="checkbox"
                      checked={on}
                      disabled={why !== null}
                      onChange={(e) => setPicked(e.target.checked ? [...chosen, d.info.id] : chosen.filter((x) => x !== d.info.id))}
                    />
                    <span>
                      <strong>{d.info.model}</strong>
                      <span className="muted">
                        {' '}
                        · {caps.channels.length} channels · up to {hz(Math.max(...caps.ratesHz))} · {d.info.id}
                      </span>
                    </span>
                  </label>
                  {why ? (
                    <span className="row">
                      <Status tone={d.holder && isAgent(d.holder) ? 'agent' : 'neutral'}>{why}</Status>
                      {d.holder && d.holder !== 'user' && (
                        <Button size="sm" variant="secondary" onClick={() => void run('lease.take', { device: d.info.id }).then(() => refresh())}>
                          Take over
                        </Button>
                      )}
                    </span>
                  ) : (
                    d.connected && <Status tone="success">connected</Status>
                  )}
                </li>
              );
            })}
          </ul>
        </div>
      ) : (
        <div className="stack">
          <fieldset className="capture-group">
            <legend className="lg-field__label">Length</legend>
            <label className="check">
              <input type="radio" name="length" checked={!length.untilStopped} onChange={() => setLength({ ...length, untilStopped: false })} />
              For a time
            </label>
            <label className="check">
              <input type="radio" name="length" checked={length.untilStopped} onChange={() => setLength({ ...length, untilStopped: true })} />
              Until I stop it
            </label>
            {length.untilStopped ? (
              <p className="lg-field__hint">Up to a day, or as much as a device holds; "Stop capture" ends it and keeps what arrived.</p>
            ) : (
              <TextField
                label="Seconds"
                type="number"
                min={0}
                step="any"
                value={String(length.seconds)}
                onChange={(e) => setLength({ ...length, seconds: Number(e.target.value) })}
              />
            )}
          </fieldset>

          {many && (
            <fieldset className="capture-group">
              <legend className="lg-field__label">Captures</legend>
              <label className="check">
                <input type="radio" name="together" checked={!together} onChange={() => setTogether(false)} />
                Each device into its own tab
              </label>
              <label className="check">
                <input type="radio" name="together" checked={together} onChange={() => setTogether(true)} />
                All into one capture
              </label>
              {asOne && (
                <TextField
                  label="Capture name"
                  hint={`Its channels follow one another, named by device (${selected.map((d) => `${d.info.model.replace(/^Virtual analyser: /, '')}: …`).join(', ')}).`}
                  value={title}
                  onChange={(e) => setGroupTitle(e.target.value)}
                />
              )}
            </fieldset>
          )}

          {asOne && (
            <fieldset className="capture-group">
              <legend className="lg-field__label">Line the devices up</legend>
              <label className="check">
                <input type="radio" name="align" checked={!alignOn} onChange={() => setAlignOn(false)} />
                At their starts
              </label>
              <label className="check">
                <input type="radio" name="align" checked={alignOn} onChange={() => setAlignOn(true)} />
                By a signal they all see
              </label>
              {alignOn ? (
                <>
                  <div className="row capture-fields">
                    {selected.map((d, k) => {
                      const caps = d.capabilities as Caps;
                      return (
                        <label key={d.info.id} className="lg-field">
                          <span className="lg-field__label">{d.info.model}</span>
                          <select
                            className="lg-input"
                            aria-label={`Shared signal on ${d.info.model}`}
                            value={alignList?.[k] ?? -1}
                            onChange={(e) => setAlignChannels((all) => ({ ...all, [d.info.id]: Number(e.target.value) }))}
                          >
                            <option value={-1} disabled>
                              Choose its channel
                            </option>
                            {caps.channels.map((ch) => (
                              <option key={ch} value={ch}>
                                {channelLabel(caps, ch)}
                              </option>
                            ))}
                          </select>
                        </label>
                      );
                    })}
                  </div>
                  <p className="lg-field__hint">
                    Wire one signal to every device: a shared clock, or any line they all see. Its rising edges line the devices up to the sample, and each device's
                    clock drift is corrected while capturing. With a steady clock the devices must start within half its period of each other; a slow clock (the virtual
                    analyser's SYNC is 100 Hz) or an irregular signal is safest.
                  </p>
                </>
              ) : (
                <p className="lg-field__hint">Close, but not to the sample: devices start a moment apart, and their clocks drift apart while capturing.</p>
              )}
              {groupIssues.length > 0 && (
                <ul className="capture-problems" aria-label="Problems with one capture">
                  {groupIssues.map((p) => (
                    <li key={p}>{p}</li>
                  ))}
                </ul>
              )}
            </fieldset>
          )}

          {many && (
            <fieldset className="capture-group">
              <legend className="lg-field__label">Sample rate</legend>
              {asOne ? (
                <p className="lg-field__hint">One capture has one rate, so it is the same for all devices.</p>
              ) : (
                <>
              <label className="check">
                <input type="radio" name="rate" checked={sameRate} disabled={rates.length === 0} onChange={() => setShared(true)} />
                The same for all devices
              </label>
              <label className="check">
                <input type="radio" name="rate" checked={!sameRate} onChange={() => setShared(false)} />
                One per device
              </label>
                </>
              )}
              {sameRate && (
                <label className="lg-field">
                  <span className="lg-field__label">Rate for all</span>
                  <select className="lg-input" value={rateShared ?? ''} onChange={(e) => setSharedRate(Number(e.target.value))}>
                    {rates.map((r) => (
                      <option key={r} value={r}>
                        {hz(r)}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              {!asOne && (
                <p className="lg-field__hint">
                  {rates.length === 0 ? 'These devices share no rate, so each gets its own. ' : ''}A device samples all its channels at one rate, so the rate is chosen
                  per device, not per channel.
                </p>
              )}
            </fieldset>
          )}

          {plans.map(({ d, caps, choice }) => {
            const mine = own({ d, caps, choice });
            const samples = samplesFor(caps, choice.rateHz, length);
            return (
              <section key={d.info.id} className="card stack-sm" aria-label={`Settings for ${d.info.model}`}>
                <strong>{d.info.model}</strong>
                {!asOne && <TextField label="Tab name" value={choice.title} onChange={(e) => setChoice(d, { title: e.target.value })} />}
                <fieldset className="channels-pick">
                  <legend className="lg-field__label">
                    Channels{' '}
                    <button type="button" className="link" onClick={() => setChoice(d, { channels: [...caps.channels] })}>
                      all
                    </button>{' '}
                    <button type="button" className="link" onClick={() => setChoice(d, { channels: [] })}>
                      none
                    </button>
                  </legend>
                  {caps.channels.map((ch) => (
                    <label key={ch} className="check">
                      <input
                        type="checkbox"
                        checked={choice.channels.includes(ch)}
                        onChange={(e) =>
                          setChoice(d, { channels: e.target.checked ? [...choice.channels, ch].sort((a, b) => a - b) : choice.channels.filter((x) => x !== ch) })
                        }
                      />
                      {channelLabel(caps as Caps, ch)}
                    </label>
                  ))}
                </fieldset>
                <div className="row capture-fields">
                  {rateShared === null && (
                    <label className="lg-field">
                      <span className="lg-field__label">Sample rate</span>
                      <select className="lg-input" value={choice.rateHz} onChange={(e) => setChoice(d, { rateHz: Number(e.target.value) })}>
                        {caps.ratesHz.map((r) => (
                          <option key={r} value={r}>
                            {hz(r)}
                          </option>
                        ))}
                      </select>
                    </label>
                  )}
                  {caps.modes.length > 1 && (
                    <label className="lg-field">
                      <span className="lg-field__label">Mode</span>
                      <select className="lg-input" value={choice.mode} onChange={(e) => setChoice(d, { mode: e.target.value === 'buffer' ? 'buffer' : 'stream' })}>
                        {caps.modes.map((m) => (
                          <option key={m} value={m}>
                            {m === 'stream' ? 'Stream (data arrives while capturing)' : 'Buffer (fill the device, then download)'}
                          </option>
                        ))}
                      </select>
                    </label>
                  )}
                  {caps.threshold && (
                    <TextField
                      label="Threshold in volts"
                      hint={`Between ${caps.threshold[0]} V and ${caps.threshold[1]} V; empty keeps the device's.`}
                      type="number"
                      step={caps.threshold[2]}
                      value={choice.thresholdVolts === null ? '' : String(choice.thresholdVolts)}
                      onChange={(e) => setChoice(d, { thresholdVolts: e.target.value === '' ? null : Number(e.target.value) })}
                    />
                  )}
                </div>
                <p className="muted">
                  {choice.channels.length} of {caps.channels.length} channels at {hz(choice.rateHz)}
                  {length.untilStopped ? `, up to ${int(samples)} samples per channel` : `, ${int(samples)} samples per channel`}.
                </p>
                {mine.length > 0 && (
                  <ul className="capture-problems" aria-label={`Problems with ${d.info.model}`}>
                    {mine.map((p) => (
                      <li key={p}>{p}</li>
                    ))}
                  </ul>
                )}
              </section>
            );
          })}
        </div>
      )}
    </Dialog>
  );
}
