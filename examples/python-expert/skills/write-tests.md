---
name: write-tests
trigger: test
---
Напиши pytest тесты. Покрой обязательно:

- **Happy path** — основной сценарий с валидными данными
- **Edge cases** — пустые коллекции, None, граничные значения
- **Error cases** — что должно бросать исключения и какие

Стиль тестов:
```python
def test_<что>_<при каких условиях>_<ожидаемый результат>():
    # Arrange
    ...
    # Act
    result = ...
    # Assert
    assert result == expected
```

Используй `pytest.mark.parametrize` для группы похожих случаев.
Моки через `unittest.mock.patch` или `pytest-mock`.
Фикстуры в conftest.py если используются в нескольких тестах.
