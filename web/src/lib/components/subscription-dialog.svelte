<script lang="ts">
	import { Button } from '$lib/components/ui/button/index.js';
	import { Input } from '$lib/components/ui/input/index.js';
	import { Label } from '$lib/components/ui/label/index.js';
	import { Checkbox } from '$lib/components/ui/checkbox/index.js';
	import { toast } from 'svelte-sonner';
	import {
		Sheet,
		SheetContent,
		SheetDescription,
		SheetFooter,
		SheetHeader,
		SheetTitle
	} from '$lib/components/ui/sheet/index.js';
	import api from '$lib/api';
	import type {
		Followed,
		InsertFavoriteRequest,
		InsertCollectionRequest,
		InsertSubmissionRequest,
		InsertBangumiRequest,
		ApiError,
		SectionInfo
	} from '$lib/types';

	interface Props {
		open: boolean;
		item: Followed | null;
		onSuccess: (() => void) | null;
	}

	let { open = $bindable(false), item = null, onSuccess = null }: Props = $props();

	let customPath = $state('');
	let loading = $state(false);
	let sections = $state<SectionInfo[]>([]);
	let selectedSectionIds = $state<number[]>([]);
	let loadingSections = $state(false);

	// 根据类型和 item 生成默认路径
	async function generateDefaultPath(): Promise<string> {
		if (!item || !itemTitle) return '';
		// 番剧不设置默认路径
		if (item.type === 'bangumi') return '';
		// 根据 item.type 映射到对应的 API 类型
		const apiType =
			item.type === 'favorite'
				? 'favorites'
				: item.type === 'collection'
					? 'collections'
					: 'submissions';
		return (await api.getDefaultPath(apiType, itemTitle)).data;
	}

	function getTypeLabel(): string {
		if (!item) return '';

		switch (item.type) {
			case 'favorite':
				return '收藏夹';
			case 'collection':
				return '合集';
			case 'upper':
				return 'UP 主';
			case 'bangumi':
				return '番剧';
			default:
				return '';
		}
	}

	function getItemTitle(): string {
		if (!item) return '';

		switch (item.type) {
			case 'favorite':
			case 'collection':
			case 'bangumi':
				return item.title;
			case 'upper':
				return item.uname;
			default:
				return '';
		}
	}

	async function handleSubscribe() {
		if (!item || !customPath.trim()) return;

		loading = true;
		try {
			let response;

			switch (item.type) {
				case 'favorite': {
					const request: InsertFavoriteRequest = {
						fid: item.fid,
						path: customPath.trim()
					};
					response = await api.insertFavorite(request);
					break;
				}
				case 'collection': {
					const request: InsertCollectionRequest = {
						sid: item.sid,
						mid: item.mid,
						path: customPath.trim()
					};
					response = await api.insertCollection(request);
					break;
				}
				case 'upper': {
					const request: InsertSubmissionRequest = {
						upper_id: item.mid,
						path: customPath.trim()
					};
					response = await api.insertSubmission(request);
					break;
				}
				case 'bangumi': {
					const request: InsertBangumiRequest = {
						season_id: item.season_id,
						path: customPath.trim(),
						selected_section_ids: JSON.stringify(selectedSectionIds)
					};
					response = await api.insertBangumi(request);
					break;
				}
			}

			if (response && response.data) {
				toast.success('订阅成功', {
					description: `已订阅${getTypeLabel()}「${getItemTitle()}」到路径「${customPath.trim()}」`
				});
				open = false;
				if (onSuccess) {
					onSuccess();
				}
			}
		} catch (error) {
			console.error(`订阅${getTypeLabel()}失败:`, error);
			toast.error('订阅失败', {
				description: (error as ApiError).message
			});
		} finally {
			loading = false;
		}
	}

	async function loadSections() {
		if (item?.type !== 'bangumi') return;

		loadingSections = true;
		try {
			const response = await api.getBangumiSections(item.season_id);
			if (response?.data) {
				sections = response.data;
				// 默认选中所有 section
				selectedSectionIds = sections.map((s) => s.id);
			}
		} catch (error) {
			console.error('加载内容列表失败:', error);
			toast.error('加载内容列表失败', {
				description: (error as ApiError).message
			});
		} finally {
			loadingSections = false;
		}
	}

	function toggleSection(sectionId: number) {
		if (selectedSectionIds.includes(sectionId)) {
			selectedSectionIds = selectedSectionIds.filter((id) => id !== sectionId);
		} else {
			selectedSectionIds = [...selectedSectionIds, sectionId];
		}
	}

	function toggleAllSections() {
		if (selectedSectionIds.length === sections.length) {
			selectedSectionIds = [];
		} else {
			selectedSectionIds = sections.map((s) => s.id);
		}
	}

	function handleCancel() {
		open = false;
	}

	$effect(() => {
		if (open && item) {
			generateDefaultPath()
				.then((path) => {
					customPath = path;
				})
				.catch((error) => {
					toast.error('获取默认路径失败', {
						description: (error as ApiError).message
					});
					customPath = '';
				});
			// 重置 sections
			sections = [];
			selectedSectionIds = [];
			// 如果是番剧，加载 sections
			if (item.type === 'bangumi') {
				loadSections();
			}
		}
	});

	const typeLabel = getTypeLabel();
	const itemTitle = getItemTitle();
</script>

<Sheet bind:open>
	<SheetContent side="right" class="flex w-full flex-col sm:max-w-md">
		<SheetHeader class="px-6 pb-2">
			<SheetTitle class="text-lg">订阅{typeLabel}</SheetTitle>
			<SheetDescription class="text-muted-foreground space-y-1 text-sm">
				<div>即将订阅{typeLabel}「{itemTitle}」</div>
				<div>请手动编辑本地保存路径：</div>
			</SheetDescription>
		</SheetHeader>

		<div class="flex-1 overflow-y-auto px-6">
			<div class="space-y-4 py-4">
				<!-- 项目信息 -->
				<div class="bg-muted/30 rounded-lg border p-4">
					<div class="space-y-2">
						<div class="flex items-center gap-2">
							<span class="text-muted-foreground text-sm font-medium">{typeLabel}名称：</span>
							<span class="text-sm">{itemTitle}</span>
						</div>
						{#if item!.type !== 'upper'}
							<div class="flex items-center gap-2">
								<span class="text-muted-foreground text-sm font-medium">视频数量：</span>
								<span class="text-sm">{item!.media_count} 条</span>
							</div>
						{:else if item!.sign}
							<div class="flex items-start gap-2">
								<span class="text-muted-foreground text-sm font-medium">个人简介：</span>
								<span class="text-muted-foreground text-sm">{item!.sign}</span>
							</div>
						{/if}
					</div>
				</div>

				<!-- 路径输入 -->
				<div class="space-y-3">
					<Label for="custom-path" class="text-sm font-medium">
						本地保存路径 <span class="text-destructive">*</span>
					</Label>
					<Input
						id="custom-path"
						type="text"
						placeholder="请输入保存路径，例如：/home/我的收藏"
						bind:value={customPath}
						disabled={loading}
						class="w-full"
					/>
					<div class="text-muted-foreground space-y-3 text-xs">
						<p>路径将作为文件夹名称，用于存放下载的视频文件。</p>
						<div>
							<p class="mb-2 font-medium">路径示例：</p>
							<div class="space-y-1 pl-4">
								<div class="font-mono text-xs">Mac/Linux: /home/downloads/我的收藏</div>
								<div class="font-mono text-xs">Windows: C:\Downloads\我的收藏</div>
							</div>
						</div>
					</div>
				</div>

				<!-- 内容选择（仅番剧） -->
				{#if item!.type === 'bangumi' && sections.length > 0}
					<div class="space-y-3">
						<div class="flex items-center justify-between">
							<Label class="text-sm font-medium">选择要下载的内容</Label>
							<button
								type="button"
								onclick={toggleAllSections}
								class="text-muted-foreground hover:text-foreground text-xs"
							>
								{selectedSectionIds.length === sections.length ? '取消全选' : '全选'}
							</button>
						</div>
						{#if loadingSections}
							<div class="text-muted-foreground text-sm">加载中...</div>
						{:else}
							<div class="space-y-2">
								{#each sections as section (section.id)}
									<div class="flex items-center gap-3 rounded-md border p-3">
										<Checkbox
											checked={selectedSectionIds.includes(section.id)}
											onCheckedChange={() => toggleSection(section.id)}
											disabled={loading}
										/>
										<div class="flex-1">
											<div class="text-sm font-medium">{section.title}</div>
											<div class="text-muted-foreground text-xs">
												{section.episode_count} 个视频
											</div>
										</div>
									</div>
								{/each}
							</div>
						{/if}
					</div>
				{/if}
			</div>
		</div>

		<SheetFooter class="bg-background flex gap-2 border-t px-6 pt-4">
			<Button
				variant="outline"
				onclick={handleCancel}
				disabled={loading}
				class="flex-1 cursor-pointer"
			>
				取消
			</Button>
			<Button
				onclick={handleSubscribe}
				disabled={loading || !customPath.trim()}
				class="flex-1 cursor-pointer"
			>
				{loading ? '订阅中...' : '确认订阅'}
			</Button>
		</SheetFooter>
	</SheetContent>
</Sheet>
