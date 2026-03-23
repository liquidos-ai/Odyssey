import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    'index',
    {
      type: 'category',
      label: 'Guides',
      items: ['guides/getting-started'],
    },
    {
      type: 'category',
      label: 'Reference',
      items: [
        'reference/cli-and-server',
        'reference/bundle-format',
        'reference/sandbox-and-tools',
      ],
    },
    {
      type: 'category',
      label: 'Runtime',
      items: [
        'runtime/runtime-model',
        'runtime/events-and-approvals',
        'runtime/architecture',
      ],
    },
  ],
};

export default sidebars;
