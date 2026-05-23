# Padrões e Diretrizes do Projeto (Ipeenv)

Bem-vindo ao repositório do Ipeenv. Este arquivo serve para orientar agentes de IA (como o Gemini) ou novos contribuidores sobre as decisões de arquitetura e padrões de código atuais do projeto.

## Arquitetura e Stack
- **Backend (Rust & Tauri)**: Localizado na pasta `src-tauri`. Todo o core do sistema (manipulação do OS, processos, geração de configs) vive aqui.
  - Ponto de entrada: `src-tauri/src/main.rs`.
  - Lógica central: `src-tauri/src/lib.rs` (Onde ficam todos os comandos do Tauri - `#[tauri::command]`).
- **Frontend (React, TypeScript & Vite)**: Localizado na pasta `src`.
  - Construído sem bibliotecas pesadas de componentes.
  - Baseado inteiramente em ícones do `lucide-react`.
  - Estado global gerido localmente nos componentes pais (`App` em `main.tsx`).
  - Chamadas ao backend feitas via `@tauri-apps/api/core` (`invoke`).
- **Estilização**:
  - Uso estrito de CSS Vanilla (`src/styles.css`).
  - Utilização agressiva de variáveis CSS (Custom Properties) para manter o suporte nativo a temas e consistência de cores sem pré-processadores.

## Convenções de Nomenclatura e Git
- **Rust**: Padrão oficial do Rust. `snake_case` para funções, variáveis e propriedades de structs. `PascalCase` para Structs e Enums.
- **TypeScript / React**: `camelCase` para variáveis e funções locais. `PascalCase` para componentes React, Types e Interfaces.
- **CSS**: `kebab-case` para classes CSS. Evitar o uso exagerado de IDs (`#`) para estilização.
- **Git Commits**: Sempre utilize **Conventional Commits** em português, porém **sem utilizar escopos**. Exemplo correto: `feat: adiciona funcionalidade x` ou `refactor: melhora componente y`. Não use `feat(ui): ...`.

## UI/UX e Padrões de Layout
- **Scrollbars**: A aplicação usa scrollbars customizados integrados ao Dark Mode global (`::-webkit-scrollbar`). Componentes longos devem conter rolagem própria.
- **Inputs**: 
  - Sempre utilize paleta Dark Mode (`var(--bg-1)`, `var(--bg-2)`) e bordas neutras (`var(--line)`) para caixas de texto.
  - Campos numéricos (`type="number"`) têm suas setas ocultas nativamente no `styles.css`.
  - Estados `disabled` ganham background mais escuro e cursor `not-allowed`.
- **Layouts em Grade (Grids CSS)**:
  - Respeite as definições de colunas de classes utilitárias base como `.mini-row` (4 colunas) ou `<dl>` (2 colunas em blocos `div` de `dt/dd`). Ao adicionar novas ações, faça o agrupamento de botões usando `Flexbox` (ex: `display: flex; gap: 4px;`) para não empurrar colunas adicionais para baixo quebrando a grid.
- **UX de Formulários**:
  - Em telas de preferências/serviços, dispense os botões monolíticos de "Salvar". Faça o salvamento de propriedades individuais ao perder o foco do campo (`onBlur`) ou ao trocar estados boleanos (`onChange`).

## Tratamento de Erros (Error Handling)
- **Rust -> React**: As funções Tauri devem retornar `Result<T, String>` ou estruturas consistentes. Muitas operações devolvem uma `ActionResult` customizada, contendo `{ ok: bool, message: string, code?: string }`.
- O Frontend captura esses erros via `try/catch` e utiliza um sistema de notificação global (`setNotice`).

## Internacionalização (i18n)
- O projeto não utiliza grandes libs de i18n como `react-i18next`.
- Em vez disso, usa um módulo nativo minimalista em `src/i18n.ts`. 
- **Sempre que adicionar novas strings ao UI**, use a função `t("Sua String")` e mapeie as traduções se necessário no arquivo `i18n.ts`.

## Regras de Refatoração
- Mantenha a dependência de pacotes (NPM) mínima no Frontend.
- Qualquer operação pesada, manipuladora de disco ou permissões deve ocorrer no backend em Rust e ser invocada no Frontend.
- Respeite o modelo de componentização monolítica atrelada a painéis da UI (`Panel`), mas, em futuras evoluções, o arquivo `main.tsx` deve ser desacoplado em múltiplos arquivos de forma gradativa.
