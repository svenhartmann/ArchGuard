//! The Python readers take foreign source: broken input must never panic.
#[path = "../../archguard-flow-common/tests/support/dirs.rs"]
mod dirs;
#[path = "../../archguard-flow-common/tests/support/variants.rs"]
mod variants;

use archguard_flow_common::extract_ctx::with_root_config;
use archguard_flow_python::{domain_python, ui_flow_django, ui_flow_fastapi, ui_flow_flask};

const APP: &str = r#"
from flask import Flask, Blueprint, render_template
from fastapi import APIRouter, Depends
from django.urls import path, include
from django.db import models

app = Flask(__name__)
bp = Blueprint("bp", __name__, url_prefix="/bp")
router = APIRouter(prefix="/api")

@app.route("/p/<int:id>", methods=["GET", "POST"])
@login_required
def view(id):
    """doc é"""
    return render_template("t.html", x=f"{id}")

@router.get("/items/{item_id}", dependencies=[Depends(auth)])
async def item(item_id: int, user=Depends(current_user)): ...

urlpatterns = [path("a/", views.a, name="a"), path("b/", include("b.urls"))]

class Order(models.Model):
    name = models.CharField(max_length=10)
    owner = models.ForeignKey("User", on_delete=models.CASCADE)
"#;

#[test]
fn python_extractors_survive_broken_files() {
    let files = [
        ("requirements.txt", "flask\nfastapi\ndjango\n"),
        ("app.py", APP),
        ("pkg/urls.py", APP),
        ("pkg/models.py", APP),
        ("pkg/views.py", APP),
        ("manage.py", APP),
    ];
    dirs::each_broken_tree(&files, |root| {
        ui_flow_fastapi::extract(root);
        ui_flow_django::extract(root);
        domain_python::extract(root);
        with_root_config(root, ui_flow_flask::extract_with);
    });
}
