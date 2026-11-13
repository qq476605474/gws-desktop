<script setup lang="ts">
//! 内嵌使用说明（详细版，带目录）。
//!
//! 实测反馈：教程在仓库的 `docs/使用教程.md` 里，装好软件的人根本找不到 ——
//! 常见问题（token 去哪申请、为什么显示未推送、需求里为什么没有文档）
//! 每次都要口头讲一遍。这里按「日常会用到」的顺序，把每个入口、每个按钮
//! 都说清楚；顶部目录一键跳到对应小节。

import { NCard, NTag } from "naive-ui";

/** 目录与小节共用一份清单，改标题时不会只改一处。 */
const sections: { id: string; title: string }[] = [
  { id: "help-what", title: "GWS 是干什么的" },
  { id: "help-start", title: "快速上手：hub 与目录结构" },
  { id: "help-new", title: "新建 / 导入需求" },
  { id: "help-list", title: "需求列表与需求详情" },
  { id: "help-daily", title: "日常：检查远程、拉取、提交、推送" },
  { id: "help-base", title: "基线：分阶段需求与「同步基线」" },
  { id: "help-env", title: "合并到环境与合并检查" },
  { id: "help-compare", title: "对比代码与浏览文件" },
  { id: "help-mr", title: "提 MR" },
  { id: "help-release", title: "快捷入口（自己配）" },
  { id: "help-docs", title: "需求文档" },
  { id: "help-repo", title: "仓库页与环境页" },
  { id: "help-paseo", title: "AI 会话（Paseo）" },
  { id: "help-settings", title: "设置、更新与 GitLab 令牌" },
  { id: "help-cli", title: "命令行 gws" },
  { id: "help-faq", title: "常见问题" },
];

/** 目录点击：滚到小节标题。用 `scrollIntoView` 而不是纯锚点 —
 *  页面滚动发生在外部容器里，纯锚点在不同浏览器下表现不一致。 */
function go(id: string) {
  document.getElementById(id)?.scrollIntoView?.({ behavior: "smooth", block: "start" });
}
</script>

<template>
  <div class="page">
    <header class="head">
      <h2 class="title">使用帮助</h2>
      <p class="muted">
        按日常顺序把每个入口、每个按钮说清楚；完整版见仓库里的
        <code>docs/使用教程.md</code>。
      </p>
    </header>

    <!-- 目录单独放在左栏并吸顶：正文很长，滚下去以后不用再滚回顶部找章节 -->
    <aside class="toc">
      <NCard size="small" class="toc-card" title="目录">
        <ol class="toc-list">
          <li v-for="s in sections" :key="s.id">
            <a :href="'#' + s.id" @click.prevent="go(s.id)">{{ s.title }}</a>
          </li>
        </ol>
      </NCard>
    </aside>

    <div class="body">
    <!-- ── 0. 这个软件是干什么的 ── -->
    <section id="help-what" class="sec">
      <NCard size="small" class="card" title="GWS 是干什么的">
        <p class="muted small">
          一个需求常常横跨七八个仓库：每个仓库开一条分支、切来切去、忘了推、
          合环境时漏了一个、提 MR 时把主干名写错……
          <strong>GWS 把「一个需求涉及的所有仓库」收进一个工作区（hub）</strong>，
          让这套流程有个固定的落脚点。
        </p>
        <ul>
          <li>
            <strong>一个需求 = 一个目录</strong>，里面按模块分文件夹 ——
            这次要改哪几个仓库，就只放哪几个。
          </li>
          <li>
            <strong>各仓库都停在这条需求分支上</strong>，所以全程不用切分支；
            拉取远程改动、合环境都是整批一起做。
          </li>
          <li>
            <strong>同一个仓库在 hub 里有多份副本</strong>（主干镜像 / 各环境 /
            当前需求），用途完全不同 —— 改代码只改需求目录那一份。
          </li>
          <li>
            合环境、提 MR、写需求文档都在需求里完成，不用来回翻网页。
          </li>
        </ul>
        <p class="muted small">
          它管的是「代码在哪儿、在哪个分支上」这件事，<strong>不替代 GitLab 和编辑器</strong> ——
          代码还是在你习惯的编辑器里改，提交也仍然由你确认。
        </p>
      </NCard>
    </section>

    <!-- ── 0. 快速上手 ── -->
    <section id="help-start" class="sec">
      <NCard size="small" class="card" title="快速上手：hub 与目录结构">
        <p class="muted small">
          <strong>hub</strong> 是一个目录，把所有仓库、需求和环境集中放在一起。
          首次使用在启动页「新建 hub」（选父目录 + 目录名 + 初始环境）；
          已有的 hub 直接「打开」，「最近打开」里能一键回到上次的 hub。
        </p>
        <pre class="tree">&lt;hub&gt;/
├─ repos/&lt;仓库&gt;/          每个仓库的主干镜像（只读，不要在这里改）
├─ ws/&lt;分支&gt;/             一个需求一个目录，每个模块一个 worktree
│   └─ <文档库>/         勾了文档库才有：这个需求的文档
├─ envs/&lt;环境&gt;/           环境镜像（dev / pre …）
├─ deployments.json       仓库 × 环境 的发布地址
└─ .gws-hub               标记这是一个 hub</pre>
        <ul>
          <li>
            需求的中文名、模块、基线、分类、备注、自定义信息都存在
            <code>ws/&lt;分支&gt;/.workspace.json</code> 里。
          </li>
          <li>
            点左上角的 hub 名字会展开菜单：<strong>改个名字 / 切换 hub / 打开 hub 目录 / 复制 hub 目录</strong>；
            「复制」给用别的 AI 平台的人——把路径粘过去当工作目录即可。右边是当前版本号。
            换 hub 不会动磁盘上的任何文件。
          </li>
          <li>长操作（同步、拉取、提交、合并、推送、同步基线）会弹进度框，逐个模块显示结果；看起来没动静时先看那里。</li>
        </ul>
      </NCard>
    </section>

    <!-- ── 1. 新建 / 导入 ── -->
    <section id="help-new" class="sec">
      <NCard size="small" class="card" title="新建 / 导入需求">
        <div class="sub">新建需求（需求页 → 新建需求）</div>
        <ul>
          <li><strong>需求名称</strong>：给人看的名字；默认 MR 标题和文档里都会用到。</li>
          <li>
            <strong>分支方式</strong>：可以写「分支前缀组合」（先选分支前缀，再填分支后缀，
            中间自动加日期），也可以直接写完整分支名。后缀里如果已经带了日期，会自动去掉，
            不会出现两个日期；日期是 8 位还是 6 位按 hub 配置。预览区显示的就是最终分支名。
          </li>
          <li>
            <strong>基线来源</strong>：留空 = 主干。分阶段需求填上游分支，如
            <code>ws:feature-20260901-上游需求</code>；可以写多条做优先级链
            （前面的优先）。也可以直接写普通分支名。
          </li>
          <li>
            <strong>需求分类</strong>：决定文档归到哪个目录。同一需求的二期 / 三期填同一个分类；
            留空按创建年月归组（如 <code>2026-10</code>），不会去猜已有分类。
          </li>
          <li>
            <strong>模块</strong>：<strong>默认只勾文档库</strong>，不会拉任何代码；需要改代码时
            再勾这次会改的仓库（不确定就少勾，之后能在详情里「增加模块」）。
            勾上文档库会一起建好文档目录。
          </li>
          <li><strong>备注 / 自定义信息</strong>：tapd、飞书文档、域名、监控链接都可以放；会显示在需求详情和仓库行里。</li>
          <li>
            创建前会列出问题：仓库不在 <code>repos/</code>、分支已在远程（可能已有需求在用）、
            本地有同名残留、基线不存在 —— 先处理再建，避免建到一半。
          </li>
          <li>创建后：<code>ws/&lt;分支&gt;/&lt;模块&gt;</code> 已经检出分支，但<strong>还没推送</strong>。</li>
        </ul>

        <div class="sub">导入需求（需求页 → 导入需求）</div>
        <ul>
          <li>
            接已经推到远程的需求：把对方「导出需求配置」的整段文本粘进来，点「识别」；
            也可以不用配置，直接填远程分支名。
          </li>
          <li>会根据「哪些仓库已有这条远程分支」自动识别涉及哪些模块。</li>
          <li>需求名称留空 = 用分支名；备注、自定义信息、需求分类可以一起带过来。</li>
          <li>同名需求已存在时会提示；只是想清掉空目录，用列表右上角的「清理残留」。</li>
        </ul>
      </NCard>
    </section>

    <!-- ── 2. 列表与详情 ── -->
    <section id="help-list" class="sec">
      <NCard size="small" class="card" title="需求列表与需求详情">
        <ul>
          <li>
            列表按<strong>需求分类</strong>分组（没填分类的按年月）；每行显示中文名、分支名、
            基线链、模块数和「模块目录缺失」提示。
          </li>
          <li>
            详情顶部：分类标签、基线、当前版本信息，以及一排操作（检查远程、拉取、提交、推送、
            合并到环境、发布、提 MR、同步基线……）；右侧按组排列：需求级
            （派发 Paseo、需求相关文档、导出需求配置）｜目录（打开目录、复制目录、
            用编辑器打开、终端）｜页面（刷新、删除需求）。
          </li>
          <li>
            <strong>删除需求</strong>在详情页右上角（危险色，二次确认），设置页不再重复放一个。
            有未保存的工作时会先列出风险项，确认后才强制删除。
          </li>
          <li>
            备注和自定义信息显示在表格上方 —— 飞书 / tapd 这类东西就是要常点，
            所以不藏在「设置」里。
          </li>
          <li>
            表格列：<strong>改动</strong>（未提交的文件数，含新文件）、
            <strong>vs远程</strong>（↑ 领先 / ↓ 落后；「未推送」= 远程还没有这条分支）、
            <strong>vs主干</strong>（+N 个提交）、操作。
          </li>
          <li>
            每个模块的按钮按组排：目录（打开目录、复制目录、用编辑器打开、终端）→
            查阅（浏览文件、对比代码、GitLab）→ 移除。用编辑器打开按设置里配置的
            常用编辑器列表，编辑器已开着会切到对应窗口；文档库固定排在第一行，同样能
            浏览 / 对比，也能从本需求「移除」（只摘掉，不删文档）。
          </li>
        </ul>
      </NCard>
    </section>

    <!-- ── 3. 日常 ── -->
    <section id="help-daily" class="sec">
      <NCard size="small" class="card" title="日常：检查远程、拉取、提交、推送">
        <ul>
          <li>
            <strong>检查远程</strong>：对每个模块 fetch 一次（只读）。有模块落后时，
            顶部会出现「远程有新提交：N 个模块落后」和「去拉取」。
          </li>
          <li>
            <strong>拉取</strong>：把远程<strong>同一分支</strong>上的提交合进来。
            默认 merge；勾「用 rebase」会改写本地提交顺序，只在确定这些提交还没合进过远程
            基于时用。只影响勾选的模块。
          </li>
          <li>
            <strong>提交</strong>：把勾选模块里<strong>所有待提交的改动</strong> add 并提交，
            适合让 AI 写完代码、你 review 完先落一个提交。提交说明默认按
            <strong>Conventional Commits</strong> 填（类型 / 影响范围 / 描述 /
            可选正文，正文一行一条、自动转成列表），影响范围直接手填、可留空，
            适用批量提交多个仓库；下面实时预览生成结果。也可以切「自由填写」，
            支持多行。描述留空就用默认说明，规范不是强制的。默认<strong>不推送</strong>；
            要一步到位就勾「提交后直接推送到远程」——有冲突或提交失败的模块不会被推。
          </li>
          <li>
            <strong>推送</strong>：把本需求分支推到远程。落后远程时会中止并提示先拉；
            只有推了，远程上才看得到，也才能合环境、提 MR。
          </li>
          <li>
            改动数 = 工作区里<strong>未提交</strong>的文件数（包括没 add 的新文件），
            不是提交数；提交过但没推的，看「vs远程 ↑N」。
          </li>
          <li>只在 <code>ws/</code> 下改代码 —— <code>repos/</code>、<code>envs/</code> 是镜像，同步时会覆盖。</li>
        </ul>
      </NCard>
    </section>

    <!-- ── 4. 基线 ── -->
    <section id="help-base" class="sec">
      <NCard size="small" class="card" title="基线：分阶段需求与「同步基线」">
        <ul>
          <li>
            普通需求基线就是主干。分阶段需求（V2 基于还没上线的 V1）把基线填成上游分支：
            <code>ws:feature-…</code>，多条时是<strong>优先级链</strong>（前面的优先）。
          </li>
          <li>
            拉代码、加模块、同步基线都按这条链找：<strong>第一个有该模块分支的用它</strong>，
            都没有才回退主干；单个仓库没有上游分支不会让整体失败。
          </li>
          <li><strong>同步基线</strong>：把基线最新提交合进本需求各模块；建议先推送，冲突处理完再推一次。</li>
          <li>
            「对比代码 → 与基线」看的就是这条链<strong>实际命中</strong>的
            <code>origin/&lt;分支&gt;</code>；退回主干时会明确标注。
          </li>
          <li>上游上线后想切回主干：在详情里编辑基线即可，不用重建需求。</li>
        </ul>
      </NCard>
    </section>

    <!-- ── 5. 合并到环境 ── -->
    <section id="help-env" class="sec">
      <NCard size="small" class="card" title="合并到环境与合并检查">
        <ul>
          <li>
            <strong>合并到环境</strong>：把本需求分支合进某个环境（dev / pre …）的本地镜像。
            默认<strong>不推送</strong>；勾「合并后直接推送到远程」远程上立刻可见。
          </li>
          <li>
            <strong>合并到环境并推送</strong>：同一个操作，只是那一项已经替你勾好。
          </li>
          <li>
            冲突会列出模块和具体文件路径，解决后再点一次同一个按钮就会自动收尾
            （已解完并 git add 的会自动补提交）。<strong>冲突现场不会被自动重置</strong> ——
            即使这期间远程环境分支又前进了，也只会提示「上次没解完，现场保留」，
            不会让你把同一个文件再解一遍。
          </li>
          <li>
            确实想放弃本地环境里的成果、按远程重来时，勾「丢弃本地未推送的合并成果，
            从远程重新合」。远程环境分支被 force push 重建过时也用这个选项。
          </li>
          <li>
            <strong>合完不用切页面</strong>：结果里直接列出发过合并的模块目录，右边
            「用编辑器打开 / 用编辑器解决冲突」就能改代码；合并检查里每个模块也有同样入口。
          </li>
          <li>
            <strong>整个环境的入口</strong>：结果列表第一行就是「dev（整个环境）」，合并检查的
            工具栏也有「打开环境目录 / 用编辑器打开整个环境」。用 Rebased、IDEA 批量 review、
            批量解冲突时直接打开环境根目录，不用一个个模块点。
          </li>
          <li>
            <strong>合并到环境检查</strong>：逐个模块给出结论 —— 有没有冲突、需求分支的提交
            是否<strong>真的都进了环境</strong>（按补丁比对，squash 也算）、环境里有没有未提交
            改动、检出分支对不对、推没推。冲突 / 漏合 / 未提交时顶部会直接报出来。
          </li>
          <li>
            每个模块下面列出<strong>已合进环境的提交</strong>：短 sha、说明、作者和提交时间，
            一眼能看出是谁什么时候合的；最多列 30 条，其余会提示数量。
          </li>
          <li>
            <strong>并入主干检查</strong>：确认各模块是否都已进主干 —— 决定能不能清理远程分支、
            这个需求是不是真的交付完了。
          </li>
          <li>合环境只认<strong>远程分支</strong>：先推送再合。</li>
        </ul>
      </NCard>
    </section>

    <!-- ── 6. 对比代码 ── -->
    <section id="help-compare" class="sec">
      <NCard size="small" class="card" title="对比代码与浏览文件">
        <ul>
          <li>
            <strong>浏览文件</strong>：文件树 + 真实内容，跟 git 状态无关，任何时候都能用。
            Markdown 文件默认直接渲染成预览（图片、表格、Mermaid 图都能看），
            右上角可切回「源码」。「需求相关文档」里的「浏览文档」也会打开同一个
            浏览窗口，所以文档可以直接在这里看，不用先下载或用外部编辑器。
          </li>
          <li>
            <strong>文档内查找</strong>：打开文件后按 <code>Cmd/Ctrl+F</code>（或点右上角
            「查找」），只查<strong>当前这份文档</strong>，跟本地编辑器一样高亮每一处、
            回车跳到下一处（Shift+回车上一处），预览和源码都能查。
            文件头会显示<strong>多少行、多少字</strong>（字数按非空白字符算）。
          </li>
          <li>
            <strong>对比代码</strong>：左边是改动文件（或提交历史、文件树），右边是
            diff / 内容；点文件还能看到最后改它的人、日期和提交说明。
            diff 默认只显示改动附近几行，需要看上下文时点右上角「完整内容」，
            会把整份文件都展开（内容多时滚动会长）。
          </li>
        </ul>
        <div class="sub">对比目标（默认「与本地」）</div>
        <ul>
          <li>
            <strong>与本地</strong>：工作区 vs 本地 HEAD，只看<strong>未提交</strong>的改动。
            分支还没推送也能用，所以是默认。
          </li>
          <li>
            <strong>与远程</strong>：本地 vs <code>origin/&lt;同名分支&gt;</code>，
            显示「本地有、远程还没有」的全部内容（含未提交）。分支没推过会提示先推送。
          </li>
          <li><strong>与主干</strong>：跟远程主干比（各仓库 main / master 自动探测）。</li>
          <li><strong>与环境</strong>：跟某个环境在这个仓库上用的分支比（先选环境）。</li>
          <li><strong>与远程分支 / 与其它本地分支</strong>：手输分支名，跟任意分支比。</li>
          <li><strong>与基线</strong>：跟需求基线链实际命中的分支比；没有需求上下文时不显示。</li>
        </ul>
        <p class="muted small">
          没被跟踪的新文件也会列出来，并合成一份「整文件新增」的 diff ——
          不会因为没 <code>git add</code> 就看不到。
        </p>
      </NCard>
    </section>

    <!-- ── 7. MR ── -->
    <section id="help-mr" class="sec">
      <NCard size="small" class="card" title="提 MR">
        <ul>
          <li>需求详情 →「提 MR」→ 勾选要提的模块（默认全选，可跳过个别模块）→ 先「预览」。</li>
          <li>预览里能看到每个模块的目标分支、评审人、改动量；没问题再「创建 MR」。</li>
          <li>
            评审人<strong>不预设</strong>：留空就是不指定评审人；要指定就按姓名或 username 搜索。
          </li>
          <li>
            标题默认 <code>需求上线-需求名</code>，描述有默认模板；<strong>两者都能改</strong>，
            改了就按你写的原样提交，留空才用默认。
          </li>
          <li>创建 MR 需要 GitLab 令牌（<code>api</code> 权限）；已有同源分支的 MR 会直接沿用。</li>
          <li>列表里能复制 MR 链接、关闭 MR、「复制清单」发给人；合并在 GitLab 里完成，这里不会自动合。</li>
        </ul>
      </NCard>
    </section>

    <!-- ── 8. 快捷入口 ── -->
    <section id="help-release" class="sec">
      <NCard size="small" class="card" title="快捷入口">
        <ul>
          <li>
            在「设置 → 快捷入口」里配（名称 + 地址，可以配多条），
            配好后会出现在总览页和需求详情里。
          </li>
          <li>
            <strong>一条都不配，界面上就不会出现快捷入口</strong>：总览页的「快捷入口」
            卡片、需求详情里的「快捷入口」按钮都会隐藏，不会留一个点了没用的入口。
          </li>
          <li>配好之后，总览页和需求详情是同一组入口；点完弹窗自动关闭。</li>
        </ul>
      </NCard>
    </section>

    <!-- ── 9. 文档 ── -->
    <section id="help-docs" class="sec">
      <NCard size="small" class="card" title="需求文档">
        <ul>
          <li>
            文档库就是一个普通 git 仓库，每个需求分支一份目录：
            <code>workspaces/&lt;需求分类&gt;/&lt;分支名&gt;/</code>。
          </li>
          <li>
            期次里的文档按用途放进子目录：<code>需求文档/</code>、<code>开发文档/</code>、
            <code>自测文档/</code>、<code>测试文档/</code>、<code>运维文档/</code>、
            <code>总体设计/</code>、<code>其他文件/</code>、<code>临时文件/</code>；
            <code>gws doc new &lt;类型&gt;</code> 会直接建到对应目录。
            AGENTS.md 只写通用约定，不写需求名 / 分类 / 期次 / 分支名，改名或换分类都不会过期。
          </li>
          <li>
            新建时勾了文档库（或详情里点「挂上文档」）就会建目录，并生成
            <code>meta.json</code>、<code>AGENTS.md</code>、<code>CLAUDE.md</code>；
            CLAUDE.md 只是指向 AGENTS.md 的指针，内容以 AGENTS.md 为准。
            想加自己的规则，写到同目录的 <code>AGENTS.local.md</code>：程序只读取、
            不覆盖，并会把它合并进生成的 AGENTS.md。
          </li>
          <li>
            详情里的「需求相关文档」直接展开这一期；只有脚手架、没有正文时，
            会明确写「该需求没有需求文档」，不会拿三个说明文件充数。
          </li>
          <li>
            文档写完在<strong>同一个弹窗</strong>里「提交并上传」（有改动时才会出现那个按钮）——
            它提交的是文档库那份工作区，并推到该需求分支；命令行等价于
            <code>gws doc sync --push</code>。
          </li>
          <li>
            「文档」页按分类 / 期次浏览；空期次不列出，本地打不开的期次也不列。
            每一期行尾有「浏览」，不用先点开右边就能直接看这一期的文档。
            首次用先「拉取文档库」，「从远程同步」拉最新。
          </li>
          <li>
            期次是不是已归档，只看<strong>有没有并入主干</strong>；没有单独的「状态」字段
            （以前手写的状态没人维护，已移除）。
          </li>
          <li>
            用 AI 写文档时，让它先读同一目录的 <code>AGENTS.md</code>，写进当前需求那一份，
            再在需求里「推送」。
          </li>
        </ul>
      </NCard>
    </section>

    <!-- ── 10. 仓库与环境 ── -->
    <section id="help-repo" class="sec">
      <NCard size="small" class="card" title="仓库页与环境页">
        <div class="sub">仓库</div>
        <ul>
          <li>「添加仓库」把 GitLab 仓库 clone 到 hub 里；只接受本 hub 绑定的那个 GitLab 上的地址。</li>
          <li>「同步主干」把各仓库镜像更新到最新主干（fetch + reset）；镜像只读，别在里面改代码。</li>
          <li>
            每行按钮按组排：目录（打开目录、复制目录、用编辑器打开）→ 查阅（浏览文件、
            对比代码）→ GitLab / 自定义信息 / 详情 / 导出配置 → 删除；
            仍被某个需求用着时会拒绝删除（环境里的是镜像，会跟着一起清掉）。
          </li>
          <li>
            发布地址按「仓库 × 环境」配置（存在 <code>deployments.json</code>），
            在每行的「详情」里编辑；那一行还能看这个仓库被哪些需求 / 环境用着。
            「自定义信息」里的备注与链接可以整份导出给对方。
          </li>
        </ul>
        <div class="sub">环境</div>
        <ul>
          <li>
            环境标识（如 <code>dev</code> / <code>pre</code>）与分支名解耦：可以给某个仓库
            单独改分支覆盖（如 <code>demo-api=dev</code>），<code>*</code> 表示所有仓库的默认。
          </li>
          <li>环境用于「合并到环境」「与环境对比」「合并检查」；改完分支覆盖后，这些入口立刻按新分支走。</li>
          <li>「同步」把某个环境镜像更新到远程；「移除」只在不再被需求使用时可用。</li>
        </ul>
      </NCard>
    </section>

    <!-- ── 11. Paseo ── -->
    <section id="help-paseo" class="sec">
      <NCard size="small" class="card" title="AI 会话（Paseo）">
        <ul>
          <li>
            需求详情<strong>顶部的「派发Paseo任务」</strong>会在 <strong>hub 根目录</strong>
            启动一个 Paseo 会话，标签页标题默认用<strong>需求名称（需求描述）</strong>，
            分支名写在开场说明里；启动后自动打开 Paseo。
          </li>
          <li>
            左下角的 <strong>「开 AI 会话」是不绑定需求的自由会话</strong>：适合建需求前先聊清需求，
            或直接让 AI 查线上日志；查出是 bug 再回「需求」页建一个来修。
            会话标题可以自己填（留空叫「临时会话」）。
          </li>
          <li>
            首次会自动把 hub 根目录注册成 Paseo 项目（已注册就跳过），
            所以 Paseo 桌面端的项目列表里能看到它。
          </li>
          <li>
            工作区会<strong>复用同一个</strong>（名称就是 hub 目录名，如 <code>my-hub</code>）：
            每次只是在里面新开一个标签页，不会在左侧堆出一排同名工作区。
            你自己命名的（如「查日志」「需求开发」）不会被占用。
          </li>
          <li>
            工作目录是 hub 根，所以 AI 读得到根目录的 <code>AGENTS.md</code>，也能看到
            <code>repos/</code> 里的主干代码。开场说明只有一行（当前分支 + 直接开始），
            不罗列要读的文件；AI 需要什么会自己按 <code>AGENTS.md</code> 去找。
          </li>
          <li>
            点按钮会先弹窗列出 <code>paseo provider ls</code> 里可用且启用的 provider
            （默认第一个），<strong>必须选一个</strong>才能启动。
          </li>
          <li>需要本机已安装 Paseo，且 <code>paseo</code> 在 PATH 或 <code>~/.local/bin</code> 里。</li>
          <li>
            <strong>没装 Paseo 时</strong>：点「开 AI 会话」不会弹 provider 弹窗，
            而是<strong>自动复制 hub 目录</strong>并提示去安装——可以把这段路径粘到
            别的 AI 平台（如 Cursor / Claude Code）的「工作目录」里直接对话。
          </li>
          <li>
            <strong>Windows：</strong>在 Paseo 桌面端「集成」→「CLI 文档」→「命令行」里
            安装 CLI，装到 <code>%USERPROFILE%\.local\bin\paseo.cmd</code>；
            <code>npm install -g @getpaseo/cli</code> 则装到
            <code>%APPDATA%\npm\paseo.cmd</code>。两个位置都会自动找，装完重启本应用。
          </li>
          <li>
            <strong>ARM 版 Windows</strong>（含 Parallels / VMware 里的 Windows 11 ARM）：
            Paseo 桌面端要下 <strong>arm64</strong> 安装包
            （<code>Paseo-Setup-&lt;版本&gt;-arm64.exe</code>），x64 包可能装不上；
            也可以直接 <code>npm install -g @getpaseo/cli</code>，CLI 不分架构。
          </li>
          <li>
            「没有可用的 provider」通常不是工具的问题：每个 provider 需要先在 Paseo 里
            装好对应 CLI 并登录（如 Claude Code / Codex），<code>paseo provider ls</code>
            里显示可用且启用才会列出来。
          </li>
        </ul>
      </NCard>
    </section>

    <!-- ── 12. 设置与更新 ── -->
    <section id="help-settings" class="sec">
      <NCard size="small" class="card" title="设置、更新与 GitLab 令牌">
        <div class="sub">外观、终端与编辑器</div>
        <ul>
          <li>主题（浅色 / 深色）；「打开终端」选跟随系统、iTerm、Terminal 等。</li>
          <li>
            「用编辑器打开」按<strong>常用编辑器列表</strong>来：点「选择程序…」从已安装的程序里挑
            （macOS 的应用、Windows 的开始菜单、Linux 的应用列表都能直接选），
            <strong>不用填任何命令</strong>，选中就记住（可多选）。
            支持上移 / 下移调整顺序、移除；程序卸载后会在设置里标「已卸载」。
            只加一个时点按钮直接打开；加多个时列出全部，点哪个用哪个；一个都没加会提示来这里添加。
          </li>
        </ul>
        <div class="sub">GitLab</div>
        <ul>
          <li>地址随 hub 绑定、建 hub 时确定、之后不可改；「连接」显示当前登录的用户。</li>
          <li>
            粘贴 <strong>Personal Access Token</strong>（勾 <code>api</code> 权限）保存；
            「去申请 Personal Access Token」直接跳到令牌页。
          </li>
          <li>
            token 存在本机配置文件里（Unix 上权限 600）；配过环境变量
            <code>GITLAB_TOKEN</code> 时它以环境变量优先。
          </li>
          <li>没有令牌也能用本地功能；但推送后改名、提 MR、查远程分支等需要它。</li>
        </ul>
        <div class="sub">版本与更新</div>
        <ul>
          <li>
            左上角<strong>一直显示当前版本</strong>；有新版会在下面出现一条单行提示
            「↑ 有新版本 …」，点它即可更新。
          </li>
          <li>
            设置页显示桌面端 / CLI / Skills 三个版本，可「同步 CLI 与 Skills」
            「检查更新」「立即更新」「打开 Releases 页」。
          </li>
          <li>
            安装 / 升级桌面端时会一并覆盖本机的 CLI 与三个 AI skill；skill 会写进本机已安装的
            AI CLI 目录（Claude Code / OpenCode / Qoder CLI / Codex），没有的目录不新建。
            改过的 skill 先备份成 <code>SKILL.md.gws-backup</code>，本机版本更新时不会降级。
          </li>
          <li>更新完成后按提示重启应用。</li>
        </ul>
      </NCard>
    </section>

    <!-- ── 13. CLI ── -->
    <section id="help-cli" class="sec">
      <NCard size="small" class="card" title="命令行 gws">
        <ul>
          <li>桌面端自带同版本的 <code>gws</code>，在 hub 目录里直接运行；不在 hub 里时加 <code>--hub &lt;路径&gt;</code>。</li>
          <li>
            常用：<code>gws ls</code>（需求列表）、<code>gws st</code>（模块状态）、
            <code>gws new</code> / <code>gws get</code>（新建 / 导入需求）、
            <code>gws add</code> / <code>gws drop</code>（增删模块）、
            <code>gws pull</code> / <code>gws push</code>、
            <code>gws merge</code>、<code>gws sync-main</code>、
            <code>gws mr</code>（预览；<code>--create</code> 才创建）、
            <code>gws done</code>、<code>gws doc</code>。
          </li>
          <li>
            <code>gws new</code> 与界面默认一致：不写 <code>--modules</code> 时<strong>只挂文档库</strong>、
            不拉代码；不写 <code>--category</code> 时按年月归组（如 <code>2026-10</code>）。
          </li>
          <li>
            内置的 AI skill 也按同一套约束写：所有字段先问用户（含选填，说明“不填用默认”），模块 / 环境 / 分类 / 评审人
            必须来自真实列表，危险操作先预览再确认。
          </li>
          <li>模块级命令都支持 <code>--only a,b</code> / <code>--exclude a,b</code>；界面上的同步、合并等也在弹窗里勾模块。</li>
          <li>
            危险操作默认<strong>只预览</strong>：<code>revert</code>、<code>reset-remote</code>、
            <code>branch-prune</code>、<code>branch-rename</code> 都要加 <code>--yes</code> 才执行。
          </li>
          <li><code>gws update</code> 检查更新，<code>gws update --apply</code> 真正替换。</li>
        </ul>
      </NCard>
    </section>

    <!-- ── 14. FAQ ── -->
    <section id="help-faq" class="sec">
      <NCard size="small" class="card" title="常见问题">
        <ul class="faq">
          <li>
            <strong>GitLab 令牌去哪儿申请？</strong>
            设置 → GitLab →「去申请 Personal Access Token」。勾选 <code>api</code> 权限，
            生成后粘回设置页保存。
          </li>
          <li>
            <strong>为什么一直显示「未推送」？</strong>
            本需求分支还只在本地。点「推送」；如果远程已有同名分支但不是你的，
            先确认分支名再操作。没推送过时「对比代码 → 与远程」没有基准，
            换成默认的「与本地」或「与主干」。
          </li>
          <li>
            <strong>为什么某个需求在「文档」页看不到？</strong>
            它只有工具生成的说明文件、没有正文。进需求详情能看到空状态提示；
            写进任意文件后就会出现在列表里。
          </li>
          <li>
            <strong>「对比代码」里的数字是什么意思？</strong>
            vs远程的 ↑/↓ 是领先 / 落后提交数；表格里的「改动」是未提交的文件数。
            列表里是本需求相对所选基准改动的文件。
          </li>
          <li>
            <strong>点「派发Paseo任务」提示找不到 paseo？</strong>
            先装 Paseo；已经装了的话确认 <code>paseo</code> 在 PATH 或
            <code>~/.local/bin/paseo</code>，然后重开桌面端再点一次。
            Windows 上是 <code>paseo.cmd</code>：在 Paseo 的「集成」→「CLI 文档」→
            「命令行」里安装，落在 <code>%USERPROFILE%\.local\bin</code>；
            npm 全局在 <code>%APPDATA%\npm</code>。
          </li>
          <li>
            <strong>弹窗说「没有可用的 provider」？</strong>
            说明 paseo 找到了，但里面没有既可用又启用的 provider。先在 Paseo 里
            装好并登录对应 CLI（Claude Code / Codex 等），再运行
            <code>paseo provider ls</code> 确认。
          </li>
          <li>
            <strong>AI 在 Paseo 里改完需求，界面没变？</strong>
            Paseo 是另一个应用，桌面端收不到通知 —— 切回来时会自动刷新
            （需求列表、打开着的详情、文档页）。窗口一直没离开过时，
            点「需求」页右上角的「刷新」。
          </li>
          <li>
            <strong>桌面端会覆盖我改过的 skill 吗？</strong>
            会按「一起自动覆盖」处理。覆盖前原文件备份成
            <code>SKILL.md.gws-backup</code>；本机版本比桌面端新时保留不降级；
            版本号相同但内容变了也会更新。
          </li>
          <li>
            <strong>推送 / 合并时提示落后于远程？</strong>
            先「拉取」（会合入远程同一分支上的提交），处理完冲突再执行原操作。
            合环境只认远程分支，所以推送要在合并之前。
          </li>
          <li>
            <strong>改了分支名，别处的工作区怎么办？</strong>
            改名会同时改本地分支、目录、文档期次，并推新删旧；别处的目录名还是旧的，
            需要删掉重新「导入需求」。已有 MR 的源分支不会自动改。
          </li>
          <li>
            <strong>能只同步 / 只操作某几个仓库吗？</strong>
            能。界面上的同步、拉取、推送、合并弹窗里都勾模块；CLI 用
            <code>--only</code> / <code>--exclude</code>。
          </li>
          <li>
            <strong>需求目录能直接删吗？</strong>
            尽量用「文档」外的正式入口或 CLI <code>gws rm</code>（会连本地分支一起清干净）；
            目录已经手工删掉的，用需求页「清理残留」扫一遍。
          </li>
        </ul>
        <div class="tags">
          <NTag size="tiny" :bordered="false">用 AI 写代码时，先让它读 hub 根目录的 AGENTS.md</NTag>
        </div>
      </NCard>
    </section>
    </div>
  </div>
</template>

<style scoped>
.page {
  display: grid;
  grid-template-columns: 210px minmax(0, 1fr);
  gap: 0 28px;
  padding: 20px 24px 40px;
  max-width: 1180px;
  align-items: start;
}
.head {
  grid-column: 1 / -1;
}
.title {
  margin: 0 0 8px;
  font-size: 18px;
  font-weight: 600;
}
/* 目录吸顶：正文滚多深都能直接跳章节 */
.toc {
  position: sticky;
  top: 12px;
  max-height: calc(100vh - 32px);
  overflow-y: auto;
}
.toc-card {
  margin-top: 16px;
}
.toc-list {
  margin: 0;
  padding-left: 18px;
  line-height: 1.8;
  font-size: 12px;
}
.body {
  min-width: 0;
}
.toc-list a {
  color: var(--n-text-color, inherit);
  text-decoration: none;
}
.toc-list a:hover {
  color: #2563eb;
  text-decoration: underline;
}
@media (max-width: 900px) {
  .page {
    grid-template-columns: minmax(0, 1fr);
  }
  .toc {
    position: static;
    max-height: none;
    overflow: visible;
  }
}
.sec {
  scroll-margin-top: 12px;
}
.card {
  margin-top: 16px;
}
.sub {
  margin: 12px 0 4px;
  font-size: 12px;
  font-weight: 600;
  opacity: 0.75;
}
.sub:first-child {
  margin-top: 0;
}
ul {
  margin: 0;
  padding-left: 20px;
  line-height: 1.8;
}
li + li {
  margin-top: 4px;
}
.muted {
  color: #888;
  font-size: 12px;
  margin: 0 0 8px;
}
.faq li {
  margin-bottom: 10px;
}
.tags {
  margin-top: 10px;
}
.tree {
  margin: 8px 0 12px;
  padding: 10px 12px;
  border: 1px solid var(--xg-border);
  border-radius: 6px;
  font-family: var(--mono);
  font-size: 12px;
  line-height: 1.7;
  overflow-x: auto;
}
code {
  font-family: var(--mono);
  font-size: 12px;
}
</style>
