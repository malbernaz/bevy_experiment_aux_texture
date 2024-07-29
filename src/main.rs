use bevy::{
    asset::load_internal_asset,
    core_pipeline::{
        core_2d::graph::{Core2d, Node2d},
        fullscreen_vertex_shader::fullscreen_shader_vertex_state,
    },
    ecs::system::lifetimeless::Read,
    prelude::*,
    render::{
        render_graph::{RenderGraphApp, RenderLabel, ViewNode, ViewNodeRunner},
        render_resource::{
            binding_types::{sampler, texture_2d},
            BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries, CachedRenderPipelineId,
            ColorTargetState, ColorWrites, FragmentState, LoadOp, MultisampleState, Operations,
            PipelineCache, PrimitiveState, RenderPassColorAttachment, RenderPassDescriptor,
            RenderPipelineDescriptor, SamplerBindingType, SamplerDescriptor, ShaderStages, StoreOp,
            TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType, TextureUsages,
        },
        renderer::RenderDevice,
        texture::{BevyDefault, CachedTexture, TextureCache},
        view::{prepare_view_targets, ViewTarget},
        Render, RenderApp, RenderSet,
    },
};

const PREPASS_SHADER: Handle<Shader> = Handle::weak_from_u128(137592469752497503917459317);
const POST_PROCESS_SHADER: Handle<Shader> = Handle::weak_from_u128(431805418475094721054309183);

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, RenderPlugin))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2dBundle::default());
}

struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, PREPASS_SHADER, "prepass.wgsl", Shader::from_wgsl);
        load_internal_asset!(
            app,
            POST_PROCESS_SHADER,
            "post_process.wgsl",
            Shader::from_wgsl
        );

        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .add_systems(
                Render,
                prepare_auxiliary_texture
                    .after(prepare_view_targets)
                    .in_set(RenderSet::ManageViews),
            )
            .add_render_graph_node::<ViewNodeRunner<TestNode>>(Core2d, TestLabel)
            .add_render_graph_edges(Core2d, (Node2d::EndMainPass, TestLabel));
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app.init_resource::<TestPipeline>();
    }
}

#[derive(Component, Deref, DerefMut)]
pub struct AuxTexture(pub CachedTexture);

pub fn prepare_auxiliary_texture(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    mut texture_cache: ResMut<TextureCache>,
    mut view_targets: Query<(Entity, &ViewTarget)>,
) {
    for (entity, view_target) in view_targets.iter_mut() {
        let texture_descriptor = TextureDescriptor {
            label: Some("auxiliary texture"),
            size: view_target.main_texture().size(),
            mip_level_count: 1,
            sample_count: view_target.main_texture().sample_count(),
            dimension: TextureDimension::D2,
            format: view_target.main_texture_format(),
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };

        let texture = texture_cache.get(&render_device, texture_descriptor);

        commands.entity(entity).insert(AuxTexture(texture));
    }
}

#[derive(Resource)]
struct TestPipeline {
    pub prepass_pipeline: CachedRenderPipelineId,
    pub post_process_pipeline: CachedRenderPipelineId,
    pub layout: BindGroupLayout,
}

impl FromWorld for TestPipeline {
    fn from_world(world: &mut World) -> Self {
        let pipeline_cache = world.resource::<PipelineCache>();

        let prepass_pipeline = pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("aux_pipeline".into()),
            layout: vec![],
            vertex: fullscreen_shader_vertex_state(),
            fragment: Some(FragmentState {
                shader: PREPASS_SHADER,
                shader_defs: vec![],
                entry_point: "fragment".into(),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState::default(),
            push_constant_ranges: vec![],
        });

        let layout = world.resource::<RenderDevice>().create_bind_group_layout(
            "post_process_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        );

        let post_process_pipeline =
            pipeline_cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("post_process_pipeline".into()),
                layout: vec![layout.clone()],
                vertex: fullscreen_shader_vertex_state(),
                fragment: Some(FragmentState {
                    shader: POST_PROCESS_SHADER,
                    shader_defs: vec![],
                    entry_point: "fragment".into(),
                    targets: vec![Some(ColorTargetState {
                        format: TextureFormat::bevy_default(),
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                }),
                primitive: PrimitiveState::default(),
                depth_stencil: None,
                multisample: MultisampleState::default(),
                push_constant_ranges: vec![],
            });

        Self {
            prepass_pipeline,
            layout,
            post_process_pipeline,
        }
    }
}

#[derive(Default)]
struct TestNode;

#[derive(RenderLabel, Clone, Hash, Debug, Eq, PartialEq)]
struct TestLabel;

impl ViewNode for TestNode {
    type ViewQuery = (Read<ViewTarget>, Read<AuxTexture>);

    fn run<'w>(
        &self,
        _: &mut bevy::render::render_graph::RenderGraphContext,
        ctx: &mut bevy::render::renderer::RenderContext<'w>,
        (view_target, aux_texture): bevy::ecs::query::QueryItem<'w, Self::ViewQuery>,
        world: &'w World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let pipeline_cache = world.resource::<PipelineCache>();
        let test_pipeline = world.resource::<TestPipeline>();
        let (Some(prepass_pipeline), Some(post_process_pipeline)) = (
            pipeline_cache.get_render_pipeline(test_pipeline.prepass_pipeline),
            pipeline_cache.get_render_pipeline(test_pipeline.post_process_pipeline),
        ) else {
            return Ok(());
        };

        let mut prepass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("prepass pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &aux_texture.default_view,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(default()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });

        // prepass
        prepass.set_pipeline(prepass_pipeline);
        prepass.draw(0..3, 0..1);
        drop(prepass);

        // post_process
        let post_process = view_target.post_process_write();

        let sampler = ctx
            .render_device()
            .create_sampler(&SamplerDescriptor::default());

        let post_process_bind_group = ctx.render_device().create_bind_group(
            "post_process_bind_group",
            &test_pipeline.layout,
            &BindGroupEntries::sequential((
                post_process.source,
                &aux_texture.default_view,
                &sampler,
            )),
        );

        let mut pass = ctx
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some("prepass pass"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: post_process.destination,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(default()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });

        pass.set_pipeline(post_process_pipeline);
        pass.set_bind_group(0, &post_process_bind_group, &[]);
        pass.draw(0..3, 0..1);

        Ok(())
    }
}
