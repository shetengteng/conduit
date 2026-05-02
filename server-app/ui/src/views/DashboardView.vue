<script setup lang="ts">
/**
 * 仪表盘视图 —— 12-column grid，遵循 data-dense dashboard 模式。
 *
 * 布局：
 *   row-1 (lg):  ProxyControl (col-span-7)  | NetworkPanel (col-span-5)
 *   row-1 (md):  ProxyControl (col-span-12) | NetworkPanel (col-span-12)
 *   row-2:       TrafficChart (col-span-12)
 *   row-3 (lg):  ClientList (col-span-8)    | ShareCard (col-span-4)
 *   row-3 (md):  ClientList (col-span-12)   | ShareCard (col-span-12)
 *
 * 优先级：
 *   - 一屏内能看到 KPI + 网络状态 + 流量曲线（720 高度场景）
 *   - 客户端表格与分享信息次屏，但 ShareCard 始终贴侧
 */
import ProxyControl from '@/components/business/ProxyControl.vue'
import NetworkPanel from '@/components/business/NetworkPanel.vue'
import TrafficChart from '@/components/business/TrafficChart.vue'
import ClientList from '@/components/business/ClientList.vue'
import ShareCard from '@/components/business/ShareCard.vue'
</script>

<template>
  <div class="mx-auto grid max-w-[1440px] grid-cols-12 gap-3 p-4">
    <div class="col-span-12 grid grid-cols-12 gap-3 lg:col-span-12">
      <div class="col-span-12 lg:col-span-7">
        <ProxyControl />
      </div>
      <div class="col-span-12 lg:col-span-5">
        <NetworkPanel />
      </div>
    </div>

    <div class="col-span-12">
      <TrafficChart />
    </div>

    <div class="col-span-12 grid grid-cols-12 gap-3 lg:col-span-12">
      <div class="col-span-12 lg:col-span-8">
        <ClientList />
      </div>
      <div class="col-span-12 lg:col-span-4">
        <ShareCard />
      </div>
    </div>
  </div>
</template>
