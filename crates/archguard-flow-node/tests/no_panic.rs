//! The Node/TypeScript readers take foreign source: broken input must never panic.
#[path = "../../archguard-flow-common/tests/support/dirs.rs"]
mod dirs;
#[path = "../../archguard-flow-common/tests/support/variants.rs"]
mod variants;

use archguard_flow_node::{domain_typescript, ui_flow_express, ui_flow_nestjs};

const TS: &str = r#"
import { Controller, Get, Post, Body, UseGuards } from '@nestjs/common';
import express, { Router } from 'express';
import { Entity, Column, OneToMany } from 'typeorm';

@Controller('orders')
@UseGuards(AuthGuard('jwt'))
export class OrdersController {
  constructor(private readonly svc: OrdersService) {}
  @Get(':id') one(@Param('id') id: string) { return this.svc.find(id); }
  @Post() @Roles('admin') add(@Body() dto: CreateOrderDto) { return this.svc.add(dto); }
}
@Entity() export class Order { @Column() name: string; @OneToMany(() => Line, l => l.order) lines: Line[]; }

const app = express();
const router = Router();
router.get('/x/:id', requireAuth, async (req, res) => { res.json(`é${req.params.id}`); });
app.use('/api', router);
app.post(`/y`, (req, res) => res.sendStatus(204));
"#;

#[test]
fn node_extractors_survive_broken_files() {
    let files = [
        (
            "package.json",
            r#"{"dependencies":{"@nestjs/core":"^10","express":"^4","typeorm":"^0.3"}}"#,
        ),
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@/*":["src/*"]}}}"#,
        ),
        ("src/app.ts", TS),
        ("src/orders.controller.ts", TS),
        ("src/server.js", TS),
        ("src/order.entity.ts", TS),
    ];
    dirs::each_broken_tree(&files, |root| {
        ui_flow_express::extract(root);
        ui_flow_nestjs::extract(root);
        domain_typescript::extract(root);
    });
}
