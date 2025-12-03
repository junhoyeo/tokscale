# Primer Design System Research

## Overview

**Primer** is GitHub's design system for building consistent, accessible, and responsive user interfaces. It consists of:

- **Primer React** (`@primer/react`) - React component library
- **Primer CSS** (`@primer/css`) - CSS framework
- **Primer Primitives** (`@primer/primitives`) - Design tokens (colors, spacing, typography)
- **Octicons** - GitHub's icon set

## React SDK

### Installation

```bash
# npm
npm install @primer/react @primer/primitives styled-components@5.x

# yarn
yarn add @primer/react @primer/primitives styled-components@5.x
```

### Basic Setup

```tsx
// Import theme CSS
import '@primer/primitives/dist/css/functional/themes/light.css'
import '@primer/primitives/dist/css/functional/themes/dark.css'

import {ThemeProvider, BaseStyles} from '@primer/react'

function App() {
  return (
    <ThemeProvider colorMode="auto">
      <BaseStyles>
        <YourApplication />
      </BaseStyles>
    </ThemeProvider>
  )
}
```

---

## Core Principles

### 1. Communication
> Design systems provide a vocabulary between design and engineering. Use words and descriptions that both designers and engineers can understand.

### 2. Innovation
- Treat everything as an experiment
- Ship often - learn from tangible outcomes
- Work on reducing complexity and simplifying solutions

### 3. Judgment
- There's often more than one way to do things
- Have **strong opinions, weakly held**
- Keep moving forward despite uncertainty

### 4. Flexibility
> Assume that people will break the rules, provide safe ways for them to do so.

Components provide styling flexibility through `className` prop while maintaining design consistency.

---

## Component Philosophy

### Component Types

| Type | Description | Examples |
|------|-------------|----------|
| **Building Blocks** | Basic components that can be combined | `Avatar`, `Details`, `Link` |
| **Pattern Components** | Standardize common UI patterns | `Button`, `Label`, `Dialog` |
| **Helper Components** | Achieve common CSS patterns | `Box`, `Stack` |

### Presentational Focus
- Primer React focuses on **presentational components**
- Components don't handle data fetching/submitting
- Create wrappers around Primer components for data handling

---

## Theming System

### Color Modes vs Color Schemes

| Concept | Values | Description |
|---------|--------|-------------|
| **Color Mode** | `day`, `night`, `auto` | Light/dark preference |
| **Color Scheme** | `light`, `dark`, `dark_dimmed` | Specific color palettes |

### ThemeProvider Configuration

```tsx
<ThemeProvider
  colorMode="auto"           // day | night | auto
  dayScheme="light"          // light | dark | dark_dimmed
  nightScheme="dark_dimmed"  // light | dark | dark_dimmed
  preventSSRMismatch         // Prevents hydration mismatch
>
```

### Accessing Theme

```tsx
import {useTheme} from '@primer/react'

function Component() {
  const {theme, colorMode, setColorMode} = useTheme()
  
  // Access theme values
  const bgColor = theme.colors.canvas.default
  const spacing = theme.space[4]
  const radius = theme.radii[2]
}
```

### Custom Color Scheme Variables

```tsx
import {useColorSchemeVar} from '@primer/react'
import {colors} from '@primer/primitives'

function Component() {
  const customBg = useColorSchemeVar({
    light: colors.light.scale.gray[1],
    dark: colors.dark.scale.gray[9],
    dark_dimmed: colors.dark_dimmed.scale.gray[2],
  })
  
  return <div style={{backgroundColor: customBg}}>...</div>
}
```

---

## Design Tokens (Primitives)

### Foreground Colors
```css
--fgColor-default     /* Primary text */
--fgColor-muted       /* Secondary text */
--fgColor-accent      /* Links, interactive */
--fgColor-success     /* Success state */
--fgColor-danger      /* Error state */
--fgColor-attention   /* Warning state */
--fgColor-disabled    /* Disabled elements */
--fgColor-onEmphasis  /* Text on emphasis bg */
```

### Background Colors
```css
--bgColor-default     /* Main background */
--bgColor-muted       /* Subtle background */
--bgColor-inset       /* Inset/sunken background */
--bgColor-emphasis    /* Strong emphasis */
--bgColor-accent-emphasis
--bgColor-success-emphasis
--bgColor-danger-emphasis
```

### Border Colors
```css
--borderColor-default
--borderColor-muted
--borderColor-emphasis
--borderColor-accent-emphasis
--borderColor-success-emphasis
--borderColor-danger-emphasis
```

### Shadows
```css
--shadow-resting-small
--shadow-resting-medium
--shadow-floating-small
--shadow-floating-medium
--shadow-floating-large
```

---

## Key Components for Token Tracker

### Avatar
```tsx
import {Avatar} from '@primer/react'

<Avatar src={user.avatarUrl} size={24} />
<Avatar src={user.avatarUrl} size={40} square />
```

### Button
```tsx
import {Button} from '@primer/react'

<Button>Default</Button>
<Button variant="primary">Primary</Button>
<Button variant="danger">Danger</Button>
<Button variant="invisible">Invisible</Button>
<Button leadingIcon={SearchIcon}>With Icon</Button>
```

### DataTable (Experimental)
```tsx
import {DataTable} from '@primer/react/experimental'

<DataTable
  data={items}
  columns={[
    {header: 'Name', field: 'name'},
    {header: 'Value', field: 'value', align: 'end'},
  ]}
  loading={isLoading}
/>
```

### Label
```tsx
import {Label} from '@primer/react'

<Label>Default</Label>
<Label variant="primary">Primary</Label>
<Label variant="secondary">Secondary</Label>
<Label variant="accent">Accent</Label>
<Label variant="success">Success</Label>
<Label variant="attention">Attention</Label>
<Label variant="danger">Danger</Label>
```

### ProgressBar
```tsx
import {ProgressBar} from '@primer/react'

<ProgressBar progress={75} />
<ProgressBar>
  <ProgressBar.Item progress={40} sx={{backgroundColor: 'success.emphasis'}} />
  <ProgressBar.Item progress={30} sx={{backgroundColor: 'attention.emphasis'}} />
</ProgressBar>
```

### Pagination
```tsx
import {Pagination} from '@primer/react'

<Pagination
  pageCount={10}
  currentPage={3}
  onPageChange={(e, page) => setPage(page)}
/>
```

### SegmentedControl
```tsx
import {SegmentedControl} from '@primer/react'

<SegmentedControl>
  <SegmentedControl.Button selected>Day</SegmentedControl.Button>
  <SegmentedControl.Button>Week</SegmentedControl.Button>
  <SegmentedControl.Button>Month</SegmentedControl.Button>
</SegmentedControl>
```

### CounterLabel
```tsx
import {CounterLabel} from '@primer/react'

<CounterLabel>12</CounterLabel>
<CounterLabel scheme="primary">99+</CounterLabel>
```

---

## Data Visualization Guidelines

### Supported Chart Types
- Bar charts
- Line charts
- Area charts
- ProgressBars

### Data Visualization Colors
```css
/* Emphasis colors (3:1 contrast) */
--data-blue-color-emphasis    /* #006edb */
--data-green-color-emphasis   /* #30a147 */
--data-purple-color-emphasis  /* #894ceb */
--data-orange-color-emphasis  /* #eb670f */
--data-red-color-emphasis     /* #df0c24 */

/* Muted colors (use with outlines) */
--data-blue-color-muted       /* #d1f0ff */
--data-green-color-muted      /* #caf7ca */
--data-purple-color-muted     /* #f1e5ff */
```

### Accessibility Requirements

| Element | Required Contrast |
|---------|------------------|
| Marks to background | 3:1 |
| Any text to background | 4.5:1 |
| Marks to other marks | N/A (use stroke styles) |

### Line Chart Best Practices
- Max 5 lines recommended
- Use different stroke styles (solid, dashed, dotted)
- Use different markers (circle, square, triangle)
- Line width: 2px minimum

---

## Integration Plan for Token Tracker

### Phase 1: Setup
1. Install dependencies
2. Configure ThemeProvider in layout
3. Import Primer CSS primitives

### Phase 2: Replace Components
| Current | Primer Equivalent |
|---------|-------------------|
| Custom skeleton | `SkeletonBox`, `SkeletonText` |
| Custom buttons | `Button`, `IconButton` |
| User avatars | `Avatar` |
| Period filter | `SegmentedControl` |
| Leaderboard table | `DataTable` |
| Pagination | `Pagination` |
| Token badges | `Label`, `CounterLabel` |

### Phase 3: Token Updates
- Replace Tailwind colors with Primer CSS variables
- Use semantic tokens (`--fgColor-default`, `--bgColor-muted`)
- Support all color schemes (light, dark, dark_dimmed)

### Phase 4: Data Visualization
- Use Primer data visualization colors for graphs
- Ensure 3:1 contrast for chart marks
- Implement proper legends and axis labels

---

## Useful Hooks

```tsx
// Theme access
const {theme, colorMode, setColorMode} = useTheme()

// Color scheme variables
const bgColor = useColorSchemeVar({light: '...', dark: '...'})

// Responsive values
const size = useResponsiveValue({narrow: 'small', regular: 'medium'})

// Confirmation dialogs
const confirm = useConfirm()
const result = await confirm({title: 'Delete?', content: 'Are you sure?'})
```

---

## Resources

- **Documentation**: https://primer.style/product/
- **React Components**: https://primer.style/product/components/
- **Primitives Storybook**: https://primer.style/primitives/storybook/
- **GitHub Repo**: https://github.com/primer/react
- **Octicons**: https://primer.style/octicons/

---

## Next Steps

1. **Decide integration depth**: Full migration vs selective adoption
2. **Choose theming approach**: CSS variables vs styled-components
3. **Plan component migration**: Start with low-risk components
4. **Consider bundle size**: Primer React adds ~50-100KB gzipped

The recommendation is to **selectively adopt** Primer components where they provide clear value (Avatar, Button, SegmentedControl, Pagination) while keeping the existing contribution graph which is already well-designed.
