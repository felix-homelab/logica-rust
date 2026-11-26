// The component gallery (doc 08; development builds only): every component, in the theme chosen
// at the top, for checking by eye. The end-to-end tests run axe on it in both themes.
import { useState } from 'react';
import {
  Badge,
  Button,
  CommandPalette,
  Dialog,
  EmptyState,
  IconButton,
  Kbd,
  Menu,
  MenuItem,
  MenuSeparator,
  Panel,
  Splitter,
  Switch,
  TabsContent,
  TabsList,
  TabsRoot,
  TabsTrigger,
  TextField,
  VirtualList,
} from '@logica/ui';
import { Copy, FolderOpen, Plus, Trash2 } from 'lucide-react';

export function Gallery() {
  const [theme, setTheme] = useState<'light' | 'dark'>('dark');
  const [on, setOn] = useState(true);
  const [dialog, setDialog] = useState(false);
  const [palette, setPalette] = useState(false);
  const [size, setSize] = useState(200);
  const set = (t: 'light' | 'dark') => {
    setTheme(t);
    document.documentElement.dataset.theme = t;
  };
  return (
    <main className="gallery pad stack">
      <div className="row between">
        <h1 className="settings__title">Component gallery</h1>
        <div className="row" role="group" aria-label="Theme">
          <Button size="sm" variant={theme === 'light' ? 'primary' : 'secondary'} onClick={() => set('light')}>
            Light
          </Button>
          <Button size="sm" variant={theme === 'dark' ? 'primary' : 'secondary'} onClick={() => set('dark')}>
            Dark
          </Button>
          <Button size="sm" variant="ghost" onClick={() => (window.location.hash = '')}>
            Back to the app
          </Button>
        </div>
      </div>

      <Panel title="Buttons">
        <div className="row pad">
          <Button variant="primary" icon={<Plus />}>
            Primary
          </Button>
          <Button>Secondary</Button>
          <Button variant="ghost">Ghost</Button>
          <Button variant="danger" icon={<Trash2 />}>
            Danger
          </Button>
          <Button disabled>Disabled</Button>
          <Button size="sm">Small</Button>
          <IconButton label="Copy" shortcut={['Ctrl', 'C']} icon={<Copy />} />
          <IconButton label="Pressed" pressed icon={<FolderOpen />} />
        </div>
      </Panel>

      <Panel title="Badges and shortcuts">
        <div className="row pad">
          <Badge>neutral</Badge>
          <Badge tone="accent">accent</Badge>
          <Badge tone="agent">claude-code</Badge>
          <Badge tone="danger">3 problems</Badge>
          <Badge tone="warning">warning</Badge>
          <Badge tone="success">no problems</Badge>
          <Kbd keys={['Ctrl', 'K']} />
        </div>
      </Panel>

      <Panel title="Fields">
        <div className="stack pad" style={{ maxWidth: 360 }}>
          <TextField label="Sample rate" hint="Up to 100 MHz with 4 channels." defaultValue="10 MHz" />
          <TextField label="Baud rate" error="115,201 is not a standard rate; the nearest is 115,200." defaultValue="115201" />
          <Switch label="Let agents connect" description="On 127.0.0.1 only." checked={on} onCheckedChange={setOn} />
          <Switch label="Disabled switch" checked={false} onCheckedChange={() => {}} disabled />
        </div>
      </Panel>

      <Panel title="Tabs and menus">
        <div className="pad stack">
          <TabsRoot defaultValue="a">
            <TabsList label="Example tabs">
              <TabsTrigger value="a">Analysis</TabsTrigger>
              <TabsTrigger value="b">Decoders</TabsTrigger>
            </TabsList>
            <TabsContent value="a">
              <p className="pad">First tab.</p>
            </TabsContent>
            <TabsContent value="b">
              <p className="pad">Second tab.</p>
            </TabsContent>
          </TabsRoot>
          <div className="row">
            <Menu label="Example menu" trigger={<Button>Menu</Button>}>
              <MenuItem icon={<FolderOpen />} shortcut={['Ctrl', 'O']} onSelect={() => {}}>
                Open…
              </MenuItem>
              <MenuItem onSelect={() => {}} disabled>
                Disabled item
              </MenuItem>
              <MenuSeparator />
              <MenuItem danger icon={<Trash2 />} onSelect={() => {}}>
                Remove
              </MenuItem>
            </Menu>
          </div>
        </div>
      </Panel>

      <Panel title="Dialogs and the palette">
        <div className="row pad">
          <Button onClick={() => setDialog(true)}>Open a dialog</Button>
          <Button onClick={() => setPalette(true)}>Open the palette</Button>
        </div>
        <Dialog
          open={dialog}
          onOpenChange={setDialog}
          title="Remove the decoder?"
          description="Its settings go too; you can undo this."
          footer={
            <>
              <Button onClick={() => setDialog(false)}>Cancel</Button>
              <Button variant="danger" onClick={() => setDialog(false)}>
                Remove
              </Button>
            </>
          }
        />
        <CommandPalette
          open={palette}
          onOpenChange={setPalette}
          items={[
            { id: '1', group: 'Actions', title: 'Open file…', shortcut: ['Ctrl', 'O'], run: () => {} },
            { id: '2', group: 'Actions', title: 'Save', disabled: true, disabledReason: 'No capture is open', run: () => {} },
            { id: '3', group: 'Captures', title: 'mystery_C.demo', hint: '10 MHz', run: () => {} },
          ]}
        />
      </Panel>

      <Panel title="Splitter, list, empty state">
        <div className="row pad" style={{ height: 160, alignItems: 'stretch' }}>
          <div className="card" style={{ width: size }}>
            {size} px
          </div>
          <Splitter orientation="vertical" label="Example width" value={size} min={120} max={400} onChange={setSize} />
          <div style={{ flex: 1, minWidth: 0 }}>
            <VirtualList count={1000} rowHeight={24} label="A thousand rows" render={(i) => <div className="pad-x lg-num">Row {i + 1}</div>} />
          </div>
        </div>
        <EmptyState icon={<FolderOpen />} title="Nothing decoded yet" actions={<Button size="sm">Add a decoder</Button>}>
          Add a decoder from the analysis to see packets here.
        </EmptyState>
      </Panel>
    </main>
  );
}
