# Product Requirements Document (PRD) - Ipeenv

## 1. Visão Geral do Produto
O **Ipeenv** é um gerenciador de ambiente de desenvolvimento web local focado em simplicidade, isolamento e performance, desenhado como uma alternativa moderna a ferramentas como Laragon e XAMPP. Ele orquestra serviços como Apache, MySQL e PHP, gerenciando automaticamente configurações complexas como VirtualHosts, arquivos `hosts` e certificados SSL locais para domínios de desenvolvimento (ex: `.test`).

## 2. Público-Alvo
Desenvolvedores web (especialmente ecossistema PHP, Laravel, WordPress) que necessitam de um ambiente local robusto e rápido de configurar em sistemas operacionais desktop (com foco primário no Windows).

## 3. Arquitetura e Stack Tecnológico
- **Frontend (UI)**: React, TypeScript, Vite. A interface é construída sem frameworks de componentes pesados, utilizando CSS Vanilla (variáveis CSS) e ícones `lucide-react` para uma UI leve, rápida e moderna. Toda a UI atual reside predominantemente em `src/main.tsx`.
- **Backend (Core/OS Integration)**: Rust (Tauri). Responsável por todo o trabalho pesado do sistema: manipulação de arquivos (VirtualHosts, `hosts`), orquestração de processos (iniciar/parar `httpd.exe`, `mysqld.exe`), chamadas de sistema, e download/instalação de pacotes.

## 4. Funcionalidades Principais (Features)

### 4.1. Gerenciamento de Serviços
- Orquestração de serviços core: **Apache** e **MySQL**.
- Arquitetura de "Habilitar/Desabilitar" serviços (separando configuração de operação).
- Detecção automática de conflito de portas no sistema (ex: avisar se a porta 80 ou 3306 já está em uso por outro app).
- Configuração customizável de portas HTTP, HTTPS e MySQL diretamente nos cards dos serviços.

### 4.2. Gerenciamento de Projetos
- Criação automatizada de projetos a partir de templates baseados no diretório `www`.
- Criação dinâmica de URLs locais terminadas em `.test`.
- Geração automática do arquivo de VirtualHost do Apache correspondente ao projeto.
- Edição automática do arquivo `hosts` do Windows para resolução DNS local.

### 4.3. SSL e HTTPS Local
- Geração de uma Autoridade Certificadora (CA) local para o Ipeenv.
- Geração e assinatura automática de certificados SSL para os domínios locais `.test` de cada projeto criado.
- Chaveamento global para habilitar/desabilitar HTTPS no Apache.

### 4.4. Gerenciamento de PHP
- Suporte a múltiplas versões do PHP instaladas simultaneamente.
- Inicialização do PHP-CGI acoplado ao Apache para processar scripts.
- Interface para habilitar/desabilitar extensões PHP atreladas a cada versão (`php.ini`).

### 4.5. Catálogo de Ferramentas e Pacotes
- Interface dedicada para instalar e gerenciar utilitários locais complementares (ex: Cmder).
- Sistema embutido de listagem, download e configuração automática baseada no backend Rust.

### 4.6. Diagnósticos e Logs
- Leitura em tempo real de logs do sistema e dos serviços.
- Diagnósticos proativos na tela inicial (avisos de binários ausentes, problemas de ambiente).

## 5. Casos de Uso
1. **Novo Projeto Rápido**: O usuário clica em "Novo Projeto", define o nome, e o sistema cria a pasta, configura o Apache, injeta no `hosts` e gera o SSL, devolvendo uma URL como `https://meuprojeto.test` pronta para uso.
2. **Troca Rápida de PHP**: O usuário necessita testar um código legado; ele altera a versão do PHP do projeto ou do sistema e reinicia os serviços através da UI.
3. **Resolução de Conflitos**: O usuário já possui o IIS rodando na porta 80. O Ipeenv avisa do conflito no dashboard. O usuário acessa a aba Serviços e altera o Apache para porta 8080.

## 6. Próximos Passos (Futuro)
- Separação de componentes do frontend (dividir o arquivo `main.tsx`).
- Suporte multiplataforma expandido (Mac/Linux).
- Adição de novos serviços plugáveis (Redis, PostgreSQL, Nginx).
